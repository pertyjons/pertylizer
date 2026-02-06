# Referensdokumentation

Denna katalog innehåller tekniska referensdokument som är relevanta för implementeringen av tracker-format-import och uppspelning i modular-synth.

---

## Dokument

### `xm-format-envelope-spec.md`
**Varför:** Det mest kompletta tekniska referensdokumentet för XM-formatets instrument- och envelope-specifikation. Innehåller exakta byte-layouter, envelope-punktformat, fadeout-formler, och sustain/loop-semantik. Direkt användbart för att verifiera att vår import-kod tolkar XM-data korrekt.

**Stödjer implementation:** Verifiering av `convert_envelope_to_adsr()`, `extract_instruments()`, `MultiPointEnvelope`, och `FadeoutRate` i projektet.

**Källor:**
- "The Complete XM module format specification v0.81" av Matti Hamalainen
- FastTracker 2 v2.04 formatdokumentation

---

### `ft2-envelope-algorithm.md`
**Varför:** 8bitbubsy's ft2-clone är den mest exakta öppen-källkods-replikationen av FastTracker 2. Koden i `ft2_replayer.c` visar den **exakta** algoritmen som FT2 använder för envelope-processering, inklusive ordningen av operationer (inkrementera → kontrollera punkt → sustain → loop → interpolera → fadeout). Kritiskt för att upptäcka subtila avvikelser i vår implementation.

**Stödjer implementation:** Avslöjar att fadeout är LINJÄR (inte exponentiell som vår implementation), att fadeout och envelope körs parallellt (inte sekventiellt), och den exakta sustain/loop-interaktionslogiken.

**Källa:** https://github.com/8bitbubsy/ft2-clone

---

### `openmpt-xm-test-cases.md`
**Varför:** OpenMPT-projektet har skapat systematiska testfall för edge cases i XM-uppspelning. Varje testfall dokumenterar ett specifikt scenario där implementationer ofta gör fel. Dessa kan användas som en checklista för att verifiera vår uppspelningsnoggrannhet.

**Stödjer implementation:** Ger konkreta testscenarier för envelope-beteende, fadeout-interaktion, NoteOff-hantering, och sustain/loop-edge cases som vi bör stödja.

**Källa:** https://wiki.openmpt.org/Development:_Test_Cases/XM

---

## Extern dokumentation (ej sparad, länkad)

- **ft2-clone fullständig källkod:** https://github.com/8bitbubsy/ft2-clone
- **OpenMPT testfall (fullständig):** https://wiki.openmpt.org/Development:_Test_Cases
- **libopenmpt (referensimplementation):** https://lib.openmpt.org/libopenmpt/
- **xmrs crate (vår XM-parser):** https://crates.io/crates/xmrs
- **MilkyTracker (alternativ referens):** https://github.com/milkytracker/MilkyTracker
