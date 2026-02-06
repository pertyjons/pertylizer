# OpenMPT XM Test Cases: Envelope-relaterade

Sammanställt från OpenMPT wiki - Development: Test Cases / XM.

**Källa:** https://wiki.openmpt.org/Development:_Test_Cases/XM

---

## Envelope-specifika testfall

### `EnvLoops.xm`
**Testar:** Envelope position inkrementeras FÖRE evaluering.
**Korrekt beteende:** Off-by-one kan uppstå om man utvärderar före inkrementering. FT2 inkrementerar först, sedan utvärderar.

### `EnvOff.xm`
**Testar:** Envelopes kan stängas av ett tick för tidigt.
**Korrekt beteende:** Envelope-avbrytning ska ske vid korrekt tick, inte ett tick för tidigt.

### `NoteOffFadeNoEnv.xm`
**Testar:** NoteOff beteende utan volume envelope.
**Korrekt beteende:** Utan volume envelope sätts volymen till 0 omedelbart vid NoteOff. Fadeout-flaggan sätts också (om inget NoteDelay).

### `NoteOffVolume.xm`
**Testar:** Volymkommando på samma rad som NoteOff.
**Korrekt beteende:** Ett volymkommando på samma rad förhindrar att volymen sätts till 0.

### `NoteOffInstrChange.xm`
**Testar:** Ensamt instrumentnummer efter NoteOff.
**Korrekt beteende:** Återställer NoteOff-status, volymen kan fortsätta.

### `Off-Porta.xm`
**Testar:** Key-off och portamento kombination.
**Korrekt beteende:** KeyOff och fadeout-flaggor ska INTE återställas vid portamento-fortsättning utan instrumentnummer.

### `OffDelay.xm`
**Testar:** NoteOff kombinerat med NoteDelay.
**Korrekt beteende:** Envelopes retriggras. Ingen fadeout sker om det finns envelope.

### `SetEnvPos.xm` (Lxx-effekten)
**Testar:** FT2-specifikt beteende för Lxx (set envelope position).
**Korrekt beteende:** FT2 sätter bara panning envelope-positionen via Lxx om volume envelope har sustain-flaggan satt. Detta är en FT2-quirk.

### `RetrigFade.xm`
**Testar:** Retrigger + fadeout interaktion.
**Korrekt beteende:** NoteOff + instrument → fadeout startar. Instrument + retrigger → fadeout ska normalt återställas. MEN vid NoteOff + instrument + retrigger på samma rad → fadeout kvarstår trots retrigger.

### `PanSustainRelease.xm`
**Testar:** Panning envelope sustain beteende.
**Korrekt beteende:** Om panning sustain nås FÖRE key-off, släpps den aldrig. Undantag: om sustain nås på exakt samma tick som key-off.

### `AutoVibratoSweepKeyOff.xm`
**Testar:** Auto-vibrato sweep i kombination med key-off.
**Korrekt beteende:** FT2 sveper auto-vibrato djup bara tills key-off. Om djupet överstiger måldjupet före key-off, spelas auto-vibrato på fullt djup vid key-off.

---

## Relevans för modular-synth

Dessa testfall exponerar subtila beteendeskillnader som påverkar hur envelopes och fadeout ska implementeras. Särskilt relevanta för:

1. **Fadeout + envelope interaktion** - fadeout och envelope körs parallellt efter key-off
2. **Sustain-beteende** - sustain pausar envelopet, inte stoppar det
3. **Loop + sustain** - sustain har prioritet över loop vid loop end
4. **NoteOff utan envelope** - annorlunda beteende än med envelope
5. **Timing-precision** - envelope position inkrementeras FÖRE evaluering
