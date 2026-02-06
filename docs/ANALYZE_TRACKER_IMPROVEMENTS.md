### **Uppdragsbeskrivning: Förbättra `analyze_tracker`-verktyget**

**Mål:**
Modifiera exempelfilen `crates/modular_synth/examples/analyze_tracker.rs` för att skapa en komplett, textbaserad representation av en hel tracker-modul (XM, MOD, S3M). Utskriften ska vara strukturerad och lättläst för både människor och AI-modeller. Den ska innehålla all information förutom rå sample-data.

**Fil att modifiera:**
`crates/modular_synth/examples/analyze_tracker.rs`

---

### **Steg-för-steg Implementation:**

#### 1. Förbättra utskrift av instrumentdetaljer

Den nuvarande `analyze_module`-funktionen skriver redan ut mycket information om instrumenten. Utöka den till att även inkludera Panning- och Pitch-envelopes.

*   I `analyze_module`-funktionen, inuti loopen som itererar över `module.instrument`:
*   Efter koden som skriver ut `Volume envelope`, lägg till liknande kodblock för `Panning envelope` och `Pitch envelope`.
*   Dessa hämtas från `instr.panning_envelope` och `instr.pitch_envelope`.
*   Skriv ut `Enabled`, `Points` (med `frame` och `value`), `Sustain enabled`, `Sustain point`, och `Loop enabled` för varje envelope, precis som för volymenvelopen.

#### 2. Skriv ut hela spellistan (Pattern Order)

Den nuvarande koden trunkerar spellistan till 32 entries. Ändra detta för att visa hela listan.

*   Hitta raden: `println!("Order: {:?}", &order[..order.len().min(32)]);`
*   Ändra den till: `println!("Order: {:?}", order);`

#### 3. Implementera komplett utskrift av alla patterns

Detta är den största ändringen. Istället för att bara visa några noter från det första patternet, ska vi iterera över och skriva ut **alla** patterns i ett format som liknar en riktig tracker.

*   **Ta bort den befintliga "First Pattern Notes"-sektionen.**
*   Skapa en ny sektion med rubriken `
=== Patterns ===`.
*   Iterera över alla patterns i modulen med `module.pattern.iter().enumerate()`.
*   För varje pattern, anropa en ny hjälpfunktion, t.ex. `print_pattern(pattern_index, pattern)`.

#### 4. Skapa hjälpfunktionen `print_pattern`

Denna funktion ska formatera och skriva ut ett enskilt pattern.

```rust
// Ungefärlig signatur
fn print_pattern(index: usize, pattern: &xmrs::prelude::Pattern) {
    // ... implementation ...
}
```

*   **Pattern-huvud:** Skriv ut ett huvud för varje pattern, t.ex.:
    `--- Pattern 0 (64 rows, 24 channels) ---`
*   **Grid-huvud:** Skriv ut ett huvud för kanalerna.
    `Row | Ch 00          | Ch 01          | ...`
*   **Iterera över rader:** Loopa från `0` till `pattern.num_rows`.
*   **Formatera varje rad:** För varje rad, skapa en sträng som representerar all data på den raden.
    *   Börja med radnumret, t.ex. `00: `.
    *   Loopa sedan igenom varje kanal (`channel_index` från `0` till `pattern.num_channels`).
    *   För varje cell (unit) på raden och kanalen, hämta `unit = &pattern.data[row_index][channel_index]`.
    *   Formatera cellens innehåll till en fast bredd, t.ex. `NOT INS VOL EFX PAR`.
        *   **NOT:** Konvertera `unit.note` till 3 tecken. T.ex. `C#5`, `---` för tom, `===` för `KeyOff`.
        *   **INS:** Konvertera `unit.instrument` till 2 tecken hex (`format!("{:02X}", ...)`). `..` om tomt.
        *   **VOL:** Konvertera `unit.volume` till 2 tecken hex. `..` om tomt.
        *   **EFX:** Konvertera `unit.effect_type` till 1 tecken (ofta 0-9, A-Z). `.` om tomt.
        *   **PAR:** Konvertera `unit.effect_param` till 2 tecken hex. `..` om tomt.
    *   Kombinera dessa till en sträng, t.ex. `C#5 01 80 A05`. Om cellen är helt tom, skriv ut `... .. .. ... ..`.
    *   Separera varje kanal med `|`.
*   **Skriv ut raden.**

**Exempel på önskad pattern-utskrift:**
```
--- Pattern 0 (64 rows, 4 channels) ---
Row | Ch 00          | Ch 01          | Ch 02          | Ch 03          |
00  | C-5 01 80 .... | ... .. .. .... | G#5 01 80 .... | ... .. .. A03  |
01  | ... .. .. .... | ... .. .. .... | ... .. .. .... | ... .. .. .... |
02  | === .. .. .... | ... .. .. .... | ... .. .. .... | ... .. .. .... |
...
```

#### 5. Skapa hjälpfunktioner för formatering

För att hålla koden ren, skapa små hjälpfunktioner inuti `print_pattern`:

*   `format_note(note: Note) -> String`: Konverterar en `xmrs::prelude::Note` till en 3-teckens sträng.
*   `format_effect(effect_type: u8, effect_param: u8) -> String`: Formaterar effekt-typ och parameter till `E.PP`-format (t.ex. `A05`).
