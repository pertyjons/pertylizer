# Guide: AtomLayout & oberoende styling av knappar i Pertylizer

Denna guide är en grundlig genomgång av hur Pertylizer kan utnyttja `egui`:s **AtomLayout** och **tuple-baserade knappar
** för att ersätta nuvarande strängformateringar (`format!`). Detta leder till:

1. **Noll heap-allokeringar** för knappar och menyer i UI-tråden.
2. **Oberoende styling** av ikoner (t.ex. färgmarkera ikonen men behålla texten i standardfärg).
3. **Renare och mer deklarativ kod**.

--- 

## 1. Vad är skillnaden? (Teori & prestanda)

Idag görs ikoner och text till en sammanhängande sträng vid varje frame:

```rust
// Före: allokerar en ny String på heapen varje frame!
ui.button(format!("{} Copy", ri::FILE_COPY_LINE))
```

Med `egui`:s Atom-system (tillgängligt i version `0.34.3`) kan en knapp ta emot en **tupel av referenser**. `egui` ritar
dessa som separata *atomer* (ikoner, text eller bilder) bredvid varandra:

```rust
// Efter: noll heap-allokeringar, mer läsbar kod!
ui.button((ri::FILE_COPY_LINE, "Copy"))
```

---

## 2. Kodrevision: egui_backend.rs

Följande ändringar kan göras direkt
i [egui_backend.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/egui_backend.rs):

### A. Toppmeny & Projektval

Peka om knappar under projektöppning/sparande till tupler:

| Radnummer | Befintlig kod (Före)                                                 | Föreslagen kod (Efter)                                     |
|-----------|----------------------------------------------------------------------|------------------------------------------------------------|
| 1646      | `.button(format!("{} New Project", ri::FILE_ADD_LINE))`              | `.button((ri::FILE_ADD_LINE, "New Project"))`              |
| 1661      | `.button(format!("{} Open Project...", ri::FOLDER_OPEN_LINE))`       | `.button((ri::FOLDER_OPEN_LINE, "Open Project..."))`       |
| 1675      | `.button(format!("{} Save Project", ri::SAVE_LINE))`                 | `.button((ri::SAVE_LINE, "Save Project"))`                 |
| 1682      | `.button(format!("{} Save Project As...", ri::SAVE_LINE))`           | `.button((ri::SAVE_LINE, "Save Project As..."))`           |
| 1700      | `.menu_button(format!("{} Recent Projects", ri::HISTORY_LINE), ...)` | `.menu_button((ri::HISTORY_LINE, "Recent Projects"), ...)` |

---

### B. Redigeringsmenyn (Klipp ut, Kopiera, Klistra in)

Förbättra menyalternativen och lägg till **oberoende färgläggning** av ikonerna (t.ex. med primär accentfärg):

```rust
// Före (rader 1858–1882):
egui::Button::new(format!("{} Copy", ri::FILE_COPY_LINE)).shortcut_text("Ctrl+C")
egui::Button::new(format!("{} Paste", ri::CLIPBOARD_LINE)).shortcut_text("Ctrl+V")
egui::Button::new(format!("{} Duplicate", ri::FILE_COPY_2_LINE)).shortcut_text("Ctrl+D")
```

```rust
// Efter (oberoende styling av ikonen):
egui::Button::new((
egui::RichText::new(ri::FILE_COPY_LINE).color(theme().colors.accent_primary),
"Copy"
)).shortcut_text("Ctrl+C")

egui::Button::new((
egui::RichText::new(ri::CLIPBOARD_LINE).color(theme().colors.accent_primary),
"Paste"
)).shortcut_text("Ctrl+V")

egui::Button::new((
egui::RichText::new(ri::FILE_COPY_2_LINE).color(theme().colors.accent_primary),
"Duplicate"
)).shortcut_text("Ctrl+D")
```

---

### C. Destruktiva knappar (Ta bort moduler/projekt)

För röda rader och knappar (t.ex. "Delete..." eller "Optimize Project") kan vi göra papperskorgs-ikonen röd, men behålla
texten läsbar och standardvit:

```rust
// Före (rad 3375-3378):
ui.button(
RichText::new(format!("{} Delete…", ri::DELETE_BIN_LINE))
.color(theme().colors.accent_red),
)
```

```rust
// Efter (endast ikonen är röd, texten behåller sin normala kontrast):
ui.button((
RichText::new(ri::DELETE_BIN_LINE).color(theme().colors.accent_red),
"Delete…"
))
```

Och för projektoptimeringen (rad 1890):

```rust
// Efter:
ui.button((
RichText::new(ri::DELETE_BIN_LINE).color(theme().colors.accent_red),
"Optimize Project"
))
```

---

## 3. Kodrevision: analyze.rs

I [analyze.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/analyze.rs) görs flera analyser och
renderingar som kan optimeras med villkorliga atomer:

### A. Rendering / Re-analyze (Rad 537–543)

Detta är ett utmärkt ställe för villkorsstyrda ikoner utan strängallokering:

```rust
// Före:
let label = if busy {
format!("{} Rendering…", ri::REFRESH_LINE)
} else {
format ! ("{} Re-analyze", ri::PLAY_LINE)
};
ui.add_enabled(can_run, egui::Button::new(label))
```

```rust
// Efter (noll allokeringar):
let atom_label = if busy {
(ri::REFRESH_LINE, "Rendering…")
} else {
(ri::PLAY_LINE, "Re-analyze")
};
ui.add_enabled(can_run, egui::Button::new(atom_label))
```

---

### B. Pin / Repin (Rad 550–556)

```rust
// Före:
let pin_label = if self .pinned.is_some() {
format ! ("{} Repin", ri::PUSHPIN_FILL)
} else {
format ! ("{} Pin", ri::PUSHPIN_LINE)
};
ui.add_enabled( self .current.is_some(), egui::Button::new(pin_label))
```

```rust
// Efter:
let pin_atom = if self .pinned.is_some() {
(ri::PUSHPIN_FILL, "Repin")
} else {
(ri::PUSHPIN_LINE, "Pin")
};
ui.add_enabled( self .current.is_some(), egui::Button::new(pin_atom))
```

---

## 4. Anpassade vyer: list_panel.rs

I [list_panel.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/list_panel.rs) ritas sidopanelernas
huvuden med manuella strängar. Vi kan rita dem med `AtomLayout` för att separera ikonen från texten färgmässigt:

```rust
// Före (rad 30–34):
ui.label(
egui::RichText::new(format!("{icon} {title}"))
.color(t.colors.text_primary)
.strong(),
);
```

I `egui 0.34.3` kan vi använda `egui::AtomLayout` direkt för att rita anpassad layout inline.

> **OBS (API-rättelse):** `AtomLayout::new` tar `impl IntoAtoms`, **inte** en `Vec<Atom>`
> — `Vec<Atom>` implementerar inte `IntoAtoms`, så `vec![...]`-varianten kompilerar
> inte. Använd en **tuple** (2–6 element stöds):

```rust
// Efter (kompilerar — tuple, inte vec!):
egui::AtomLayout::new((
egui::RichText::new(icon).color(t.colors.accent_primary),
egui::RichText::new(title).color(t.colors.text_primary).strong(),
))
.show(ui);
```

Detta separerar logiskt och estetiskt ikonen (som får accentfärg) från titeln.
Notera också att `ui.label(...)` bara tar `Into<WidgetText>`, inte `IntoAtoms` — för
en flercells-rad krävs `AtomLayout` eller `ui.add(Label::new((a, b)))`.

---

## 5. Atomer för patch-moduler & widgets (den verkliga vinsten)

Menyknapparna ovan är den lågt hängande frukten, men störst nytta gör Atoms i
**patch-modulerna** och deras parameter-widgets. Två saker måste vara klara först:

**Vad Atoms ÄR:** en horisontell rad-primitiv för tre sorters celler — text, bild,
och egen-målad yta (`Atom::custom`) — med automatisk baslinje-/vertikal centrering,
*ett* enhetligt mellanrum (`gap`), per-cell färg, samt grow/shrink/truncation.

**Vad Atoms INTE är:** de ritar inte knob-ratten, envelope-kurvan, scope, mätare
eller kablar. Det är handmålad canvas (`painter.circle/line/text`) och förblir det.
Atoms hjälper **kompositionen och etiketteringen runt** widgets, inte DSP-grafiken inuti.

Tre konkreta ställen i koden, rangordnade efter nytta/risk:

### A. Label + mod-marker-raderna i `param_grid.rs` (störst vinst, minst risk)

Samma mönster upprepas fyra gånger (waveform, slider, dropdown, toggle):

```rust
// Före:
ui.horizontal( | ui| {
ui.label(RichText::new( & param.name).size(...).color(text_secondary));
if let Some(role) = mod_role(param) { draw_mod_marker(ui, role); }
});
```

Idag bär `ui.horizontal` + default-spacing risken att markörens baslinje inte ligger
i linje med texten. Med Atoms blir det en cellrad med korrekt centrering och en gap
från `theme()`:

```rust
// Efter:
let mut atoms = egui::Atoms::new(
egui::RichText::new( & param.name).color(theme().colors.text_secondary),
);
if let Some(role) = mod_role(param) {
atoms.push_right(
egui::RichText::new(role.glyph()).color(theme().colors.accent_purple),
); // tooltip läggs på responsen efteråt
}
egui::Label::new(atoms).selectable(false).ui(ui);
```

Vinsten är inte prestanda — det är att markören **alltid sitter rätt mot texten**, med
samma gap överallt, definierat på ett ställe.

### B. Modul-headern via en custom-atom (tar bort manuell rect-matte)

`draw_module_header` allokerar idag en 4×16-rekt för accent-stapeln för hand och
gissar den vertikala justeringen mot titeln. `Atom::custom` reserverar cellen, egui
centrerar den mot titeltexten, och du målar bara pixlarna:

```rust
let bar = egui::Id::new("accent_bar");
let resp = egui::AtomLayout::new((
egui::Atom::custom(bar, egui::vec2(4.0, 16.0)), // egui centrerar mot titeln
egui::RichText::new(title).strong().size(13.0).color(accent_color),
))
.sense(egui::Sense::click()) // klick-att-byta-namn behålls
.show(ui);
if let Some(rect) = resp.rect(bar) {
ui.painter().rect_filled(rect, 2.0, accent_color);
}
```

Med `Atom::grow()` mellan titel och `actions` trycks knapparna ut till höger och
titeln kan trunkeras snyggt med `…` i smala paneler — deklarativt i stället för
bredd-matte. Gradientwashen och `ui.separator()` förblir egen painter.

### C. Knoben — begränsad nytta (skriv inte om)

Knoben är ~95 % handmålad (båge, indikatorprick, hörn-marker, etikett). Atoms
ersätter inget av det; vinsten vore marginell. **Skriv inte om knob/envelope/scope för
Atoms skull** — fel verktyg för canvas-widgets. Däremot är `Atom::custom` superkraften
för *nya* små inline-element (status-prick före modulnamn, färg-swatch, mini-mätare i
en textrad): du får egui:s justering/spacing/trunkering gratis och målar bara innehållet.

---

## 6. Prioriterad ordning — börja smått

Bevisa mönstret på minsta möjliga yta innan det sveps brett. Varje steg ska byggas
med `cargo build && cargo clippy --all-targets && cargo test` grönt och ögnas i appen.

| # | Steg                                                        | Yta                                            | Risk       | Varför här                                                                                   |
|---|-------------------------------------------------------------|------------------------------------------------|------------|----------------------------------------------------------------------------------------------|
| 1 | **Meny-/projektknappar → tupler** (sektion 2A)              | `egui_backend.rs`, ~5 call-sites               | Mycket låg | Rena `(icon, "text")`-byten, ingen styling-ändring — verifierar att API:t sitter             |
| 2 | **Oberoende ikon-färg i Edit-menyn** (2B/2C)                | `egui_backend.rs`, Copy/Paste/Duplicate/Delete | Låg        | Första gången ikon och text färgsätts separat; liten, synlig vinst                           |
| 3 | **`labeled_atoms()`-helper + de 4 param_grid-raderna** (5A) | `param_grid.rs`                                | Låg        | Störst återanvändning per rad; rättar baslinjejusteringen i hela patch-editorn på ett ställe |
| 4 | **Villkorliga knapp-labels** (3A/3B)                        | `analyze.rs` Re-analyze/Pin                    | Låg        | Tar bort per-frame `format!` i en het re-analyze-loop                                        |
| 5 | **Modul-headern → AtomLayout + custom-atom** (5B)           | `frame.rs` `draw_module_header`                | Medel      | Rör delad framing (patch + mixer); gör efter att helpern i steg 3 bevisat mönstret           |
| 6 | **Sidopanel-huvuden** (sektion 4)                           | `list_panel.rs`                                | Medel      | Beror på samma AtomLayout-mönster som steg 5; gör sist                                       |

**Uttryckligen utanför scope:** knob/envelope/scope/cables (handmålad canvas — se 5C).

**Tumregel mellan stegen:** stanna efter steg 1–2, bygg och ögna. Om baslinje/gap ser
rätt ut, fortsätt till 3. Helpern i steg 3 är vändpunkten — när den känns bra är 5 och
6 bara samma mönster på fler ställen.

---

## 7. Konkret exempel: context-barsens ojämna labels & dropdowns

De omgjorda topbarsen (patch-baren m.fl.) har labels, dropdowns och drag-fält som inte
riktigt ligger i linje. Det här är ett bra skarpt exempel eftersom det visar **både var
atoms hjälper och var de inte är rätt verktyg.**

### Varför det ser ojämnt ut

Varje bar byggs av `toolbar::top` → `toolbar::row`, som lägger allt i **en enda
`ui.horizontal`-rad** ([toolbar.rs:51](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/toolbar.rs)).
egui centrerar varje widget vertikalt mot radens högsta element, men de tre sorternas
element har olika *intern* vertikal padding:

- **`ui.label("Vol")`** — ren text, ingen ram → baslinjen hamnar på en höjd.
- **`DragValue`** — text i en ramad widget med `button_padding.y`.
- **`ComboBox`** — ramad *plus* pildikon, default lite högre (`interact_size 40×18`).

Bounding-boxarna centreras, men **text-baslinjerna** inuti dem landar på olika y — så de
fristående labelsen ("Ch", "Vol", "Pan"…) flyter i förhållande till värdena. Det har
inget med `format!` att göra.

### Viktigt: AtomLayout radar INTE upp baren

`AtomLayout`/`Atoms` justerar innehåll **inuti en enda widget**, inte syskon-widgets i en
rad. Du kan alltså **inte** wrappa hela baren i en AtomLayout för att rada upp en
`ComboBox` mot en `ui.label` — combos och dragvalues är fulla egui-widgets, inte atomer.

### Vinst 1 — fäll in drag-labels i `DragValue::prefix()` (riktig atom-vinst)

`DragValue::prefix()` tar `impl IntoAtoms` i 0.34.3. Labels framför drag-fälten — i
patch-baren **Vol, Pan, Tr, Voices, Vel→A, Vel→F** — kan flytta *in* i fältet:

```rust
// Före: två separata widgets, olika höjd/baslinje
ui.label(RichText::new("Vol").color(theme().colors.text_dim));
ui.add(egui::DragValue::new(&mut vol).range(0.0..=1.0).speed(0.005)
    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));

// Efter: label är en prefix-atom i samma ram → garanterat baslinjejusterad
ui.add(egui::DragValue::new(&mut vol).range(0.0..=1.0).speed(0.005)
    .prefix(egui::RichText::new("Vol ").color(theme().colors.text_dim))
    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
```

Tar bort de flytande labelsen och gör varje "label+värde" till en sammanhållen cell.

### Vinst 2 — combos: enhetlig höjd i `toolbar::row`, inte atoms

`ComboBox::selected_text` tar bara `Into<WidgetText>` (ingen prefix-atom; combon har bara
`.icon()` för pilen). Combo-labels (Ch/OS/SC) och Category/Mode/Stealing/Sidechain kan
alltså inte sluka sin label som atom. Fixen är höjd-/baslinjekonsistens, gjord **en gång**
i den delade raden så att **alla åtta barerna** rättas samtidigt:

```rust
// toolbar.rs, i row():
ui.spacing_mut().interact_size.y = 20.0;          // alla ramade widgets samma höjd
ui.spacing_mut().button_padding = egui::vec2(6.0, 2.0); // combo & drag matchar
```

### Detta gäller fler topbars

Alla context-bars går genom `toolbar::row`, så höjd-tweaken (Vinst 2) träffar samtliga,
och prefix-mönstret (Vinst 1) passar varje `label → DragValue`-par i dem:

| Bar | Fil | Kandidater |
|-----|-----|-----------|
| Patch | `egui_backend.rs:2114` | Vol/Pan/Tr/Voices/Vel→A/Vel→F → prefix; Ch/OS/SC-combos → höjd |
| Mixer | `mixer_view.rs:282` | gain/pan-drag → prefix |
| Sample | `sample_view.rs:330` | drag-fält → prefix |
| Pattern | `pattern_view.rs:190` | längd/repeat-drag → prefix |
| AWE | `awe_view.rs:602` | parameter-drag → prefix |
| Transport | `sequencer/mod.rs:1114` | BPM/sig-drag → prefix |
| Piano-roll (+ secondary) | `piano_roll.rs:2524/2752` | grid/snap-drag → prefix |

> **Verifiera kandidaterna per bar** innan ändring — listan ovan är var man letar, inte en
> garanti att varje fält är en `DragValue` (vissa är combos/knappar och faller då under Vinst 2).

### Var i prio-listan

- **Vinst 2 (höjd-tweak i `toolbar::row`)** är en ~2-raders ändring som rättar alla barer
  på en gång → gör den **tidigt** (kan ligga som eget steg 0/2.5, mycket låg risk).
- **Vinst 1 (prefix-atomer)** görs per bar och bevisas lämpligen först på **patch-baren**,
  sedan rullas ut till de övriga i tabellen — samma mönster, en bar i taget.
