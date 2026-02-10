# Tracker-experiment: Sammanfattning

## Vad vi försökte

Under v0.87–v0.98 (2 dagar, 9 buggfixar) försökte vi importera och spela upp tracker-moduler (XM/MOD/S3M) via `xmrs`-craten. Målet var att konvertera tracker-data till vår interna Song/Pattern-modell och spela upp med synth-motorn.

## Varför det inte fungerade

**Arkitektur-mismatch**: Vår synth-motor är polyfonisk och semitone-baserad, medan tracker-uppspelning kräver:
- **Period-baserad pitch** med Amiga-periodberoende effekter
- **Tight-kopplad effektprocessering** (tick-för-tick med specifik ordning)
- **Kanal-baserad mono-röstallokering** (en röst per kanal, inte polyfoni)
- **Exakta tick-timing** för effekter som vibrato, portamento, arpeggio

Varje bugfix avslöjade nya buggar. Problem som fixades (v0.90–v0.98):
1. Volymberäkning helt fel (0.90)
2. Pitch-beräkning: period → Hz-konvertering (0.91)
3. Note-off vid volym 0 (0.92)
4. Tone portamento target försenad en rad (0.94)
5. pitch_offset-läcka mellan effekter (0.95)
6. Extra effekt-tick per rad (25% drift) (0.96)
7. TonePortamento 4x för långsam i Amiga-läge (0.97)
8. Vibrato/arpeggio vid tick 0 (0.98)

## Forskningsresultat

Alla existerande implementationer renderar egen PCM — ingen separerar data-import från uppspelning:
- **libxm** (C): Komplett XM-spelare, renderar direkt till float-buffer
- **libopenmpt** (C++): Multi-format, renderar direkt
- **xmrsplayer** (Rust): Använder xmrs-data men har egen renderer
- **ft2play/ft2-clone** (C): Exakt FT2-replikering, tight-kopplad effekt+render

## Alternativ som identifierades

1. **Period-baserad refaktorering** — Skriv om hela pitch-systemet till period-baserat. Enormt scope.
2. **ft2play-port** — Porta ft2play.c till Rust. Fungerar men ger oss en separat spelare, inte integration.
3. **xmrsplayer-inbäddning** — Bädda in xmrsplayer som PCM-källa. Fungerar men ingen synth-integration.

## Slutsats

Tracker-uppspelning och polyfonisk synth är fundamentalt olika problem. Att försöka göra båda i samma motor leder till en kompromiss som varken spelar tracker-moduler korrekt eller är en bra synth.

**Beslut:** Separera spåren helt. Ta bort ALL tracker-funktionalitet och fokusera på en ren, modulär synth.

Koden finns taggad som `v0.98.0-tracker-experiment` för referens.
