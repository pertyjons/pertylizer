# XM-uppspelningsbuggar - Analysrapport

## Sammanfattning

Analys av XM-moduluppspelning identifierade **3 kritiska buggar**, **5 medelallvarliga** och **3 mindre** problem. De tre kritiska buggarna förklarar de rapporterade symptomen: fel volym, saknade effekter, och generellt "konstigt" ljud.

---

## KRITISK BUGG 1: Volymen appliceras DUBBELT (velocity²)

**Filer:** `sequencer_engine.rs:662-667`, `voice.rs:626-633`

**Problemet:** När en not triggas med explicit instrument, används notens velocity (volymkolumn-värdet) som *både* tracker-volym *och* voice-velocity. Dessa multipliceras sedan:

```
amp_scale = velocity_scale * tracker_volume
          = velocity * velocity
          = velocity²
```

**Flödet:**
1. Import: XM-volymkolumn (0-64) -> `Cell.velocity` (0.0-1.0)
2. Sequencer (`sequencer_engine.rs:666`): `InstrumentDefaults.volume = velocity.as_f32()`
3. Effektprocessor: `ChannelEffectState.volume = 0.5` -> skickas som `tracker_volume`
4. Voice (`voice.rs:627-629`): `velocity_scale = 0.5`, `tracker_vol = 0.5`, `amp_scale = 0.25`

**Resultat:** En not med volym 32 (50%) spelas med 25% amplitud. En not med volym 48 (75%) spelas med 56%. Allt blir *mycket* tystare an avsett, och dynamiken komprimeras.

**Korrekt XM-beteende:** Volymkolumnen satter *kanalvolymen*, inte velocity. Voice velocity bor vara 1.0 for tracker-moduler. Kanalvolymen hanteras enbart av effektprocessorn.

**Fix:** I tracker-lage, satt velocity till 1.0 for NoteOn-events och lat effektprocessorn hantera all volymkontroll via `tracker_volume`.

---

## KRITISK BUGG 2: tracker_panning lagras men anvands ALDRIG

**Filer:** `voice.rs:284`, `synth_engine.rs:1793`, `instrument.rs:922`

**Problemet:** Alla panneringseffekter (8xx, Pxy) beraknas korrekt av effektprocessorn och lagras i `voice.tracker_panning`, men **lasas aldrig** under ljudrendering.

- `voice.process_audio()` applicerar `amp_scale` men ingen pannering
- `instrument.stereo_gain()` anvander enbart instrument-niva `self.pan`, inte per-voice-pannering
- Alla voices i samma instrument spelas fran samma panoramaposition

**Resultat:** All kanalspecifik pannering ignoreras. 8xx-effekter (set panning), Pxy-effekter (panning slide), och samplarnas default-pannering har **ingen horbar effekt**. Istallet for stereo-separation (som i OpenMPT/Schism Tracker) spelas allt fran center eller instrumentets statiska pan.

**Fix:** I `voice.process_audio()`, applicera `tracker_panning` pa `left_out`/`right_out` via constant-power panning, *innan* instrument-niva mix.

---

## KRITISK BUGG 3: Instrument-defaults saknar verkliga sample-varden

**Fil:** `sequencer_engine.rs:662-668`

```rust
// TODO: Get actual sample defaults from instrument mapping.
// For now, use velocity as volume (which is how tracker formats work).
Some(InstrumentDefaults {
    volume: NormalizedValue::new(velocity.as_f32()),
    panning: BipolarValue::CENTER,  // ALLTID CENTER!
})
```

**Tva problem:**
1. **Volym**: Anvander notens velocity istallet for samplets `default_volume`. XM-specen sager: "vid ny not med instrument, aterstall kanalvolymen till samplets default_volume"
2. **Pannering**: Hardkodad till CENTER. Samplets `default_panning` ignoreras helt. XM-specen: "instrument numbers in patterns always reset the channel's panning to the current sample's initial panning"

**Fix:** Skicka samplets `default_volume` och `default_panning` fran instrument-konfigurationen till `InstrumentDefaults` istallet for att anvanda velocity/hardkodad CENTER.

---

## MEDEL BUGG 4: Tick 0 modulering skickas inte till voice

**Fil:** `sequencer_engine.rs:332-350`

`process_row_start()` bearbetar tick 0 effekter (SetVolume, SetPanning, etc.) och uppdaterar kanalstaten. Men ingen `SequencerEvent::Modulation` emitteras for tick 0. Modulations-events skapas forst av `process_tick()` (tick 1+).

**Resultat:** Voice:ns `tracker_volume` och `tracker_panning` uppdateras forst ~1ms efter rad-start. SetVolume-effekter pa tick 0 har en fordrojning. Detta kan orsaka korta klick/glitchar vid volymforandringar.

**Fix:** Emittera en `Modulation`-event for tick 0 direkt efter `process_row_start()`, eller lat NoteOn-eventet bara initial modulation.

---

## MEDEL BUGG 5: `is_significant()` filtrerar bort nodvandiga moduleringar

**Fil:** `tracker_effects.rs:1397-1405`

```rust
pub fn is_significant(&self) -> bool {
    self.pitch_cents.as_f32().abs() > 0.01
        || (self.volume.as_f32() - 1.0).abs() > 0.01
        || self.panning.as_f32().abs() > 0.01
        || self.note_triggered || self.note_cut
        || self.sample_offset.as_u16() > 0
        || self.tone_porta_pitch.is_some()
}
```

Om en not triggas med volym 1.0 och center-pannering, emitteras **ingen** modulering. Om foregaende not hade volym 0.3, behaller voice den gamla `tracker_volume = 0.3` tills en "signifikant" modulering skickas.

**Fix:** Emittera alltid modulering pa forsta ticken efter en ny rad, oavsett varden. Alternativt, emittera modulering nar en not triggas (via `note_triggered` flaggan, som redan ar signifikant).

---

## MEDEL BUGG 6: XM Arpeggio-ordning (FT2-quirk)

**Fil:** `tracker_effects.rs:1301-1308`

Koden implementerar ProTracker-ordning (0, x, y):
```rust
match self.current_tick.as_u8() % 3 {
    0 => 0,          // bas
    1 => semitone1,  // x (forsta param)
    _ => semitone2,  // y (andra param)
}
```

Men FT2 spelar arpeggion **baklanges**: (0, y, x).
Referens: OpenMPT Wiki - "FT2's arpeggio is notably buggy, playing notes backwards".

**Fix:** For XM-filer, byt ordning till (0, y, x). Behall (0, x, y) for MOD-filer.

---

## MEDEL BUGG 7: PatternDelay inte implementerat

**Fil:** `sequencer_engine.rs:726-729`

```rust
GlobalCommand::PatternDelay(_rows) => {
    // Pattern delay is complex - would need row-level tracking
    // For now, this is not implemented
}
```

XM-moduler som anvander `EEx` (pattern delay) kommer spela med fel timing.

**Fix:** Implementera radfordrojning i sequencer-motorn. Vid PatternDelay(n) upprepas nuvarande rad n extra ganger innan nasta rad processas.

---

## MEDEL BUGG 8: Portamento-skalning kan vara fel

**Fil:** `tracker.rs:741-749`, `tracker_effects.rs:762`

Import: `scaled = speed.abs() * 16.0` -> Effektprocessor: `portamento_speed = speed * 4.0 cents`

Total skalning: `original_xmrs_speed * 16.0 * 4.0 = original * 64 cents/tick`

I XM linjart frekvensalge: 1 semitone = 64 period-enheter, portamento glider med `speed * 4` perioder/tick. Om xmrs normaliserar `speed` till "perioder per tick / 4", stammer formeln. Men utan bekraftelse fran xmrs-dokumentation kan skalningen vara fel, vilket ger for snabb/langsam portamento.

**Fix:** Verifiera xmrs normalisering genom att jämföra med OpenMPT for en testmodul. Justera skalningsfaktorn om nodvandigt.

---

## LITEN BUGG 9: EffectWaveform::Random ar deterministisk

**Fil:** `effects.rs:182-185`

```rust
let seed = (phase * 1000.0) as u32;
let hash = seed.wrapping_mul(2_654_435_761);
```

Samma fas ger alltid samma "slumpmassiga" varde. FT2 genererar nya slumpvarden varje tick.

**Fix:** Anvand ett enkelt LFSR eller xorshift-tillstand som uppdateras vid varje tick, istallet for hash pa fasen.

---

## LITEN BUGG 10: LoopMode::Backward i SamplePlayer fungerar inte

**Fil:** `sample_player.rs` - `advance_position()`

`note_on()` satter alltid `direction = Forward`. `LoopMode::Backward`-grenen i `advance_position()` nas bara om position < loop_start vid bakatuppspelning. Men ingen kod byter till bakatriktning utom PingPong. Backward loop-mode ar effektivt dead code.

**Fix:** For `LoopMode::Backward`, satt `direction = Backward` i `note_on()` och starta position vid `loop_end`.

---

## LITEN BUGG 11: PingPong-overshoot forloras

**Fil:** `sample_player.rs` - `advance_position()`

Forward-loop bevarar overshoot noggrant: `loop_start + overshoot.rem_euclid(loop_len)`. PingPong satter position till `loop_end - 1.0` och forlorar overskjutning, vilket orsakar subtila timing-fel vid varje studs.

**Fix:** Berakna overshoot och applicera den i omvand riktning vid riktningsbyte.

---

## Prioriteringsordning for fixar

| Prio | Bugg | Paverkan |
|------|------|----------|
| **1** | Velocity^2 (dubbel volym) | Allt for tyst, dynamik forstord |
| **2** | tracker_panning ignoreras | All stereo-separation saknas |
| **3** | Fel instrument-defaults | Volym/pan aterstalls till fel varden |
| **4** | Tick 0 modulering saknas | Korta glitchar vid radgranser |
| **5** | is_significant() filter | Stale modulations-varden |
| **6** | Arpeggio-ordning | Arpeggion spelar fel intervall |
| **7** | PatternDelay saknas | Timing-fel i moduler med EEx |

## Referenser

- [XM Format Specification (GitHub Gist)](https://gist.github.com/loveemu/737ace92f08b439a416adc829ae2aa76)
- [MilkyTracker Effects Commands](https://battleofthebits.com/lyceum/View/Milkytracker+Effects+Commands)
- [OpenMPT Compatible Playback Reference](https://wiki.openmpt.org/Manual:_Compatible_Playback)
