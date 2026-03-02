# Implementeringsplan: Modulgrupper

> Status: FAS 1-2 KLARA | Uppdaterad: 2026-03-02 | Implementerad i: v0.193.0–v0.197.0

## Innehåll

1. [Vision](#1-vision)
2. [Arkitektur](#2-arkitektur)
3. [Fas 1: Visuell gruppering — KLAR](#fas-1-visuell-gruppering--klar)
4. [Fas 2: Sparbara grupptemplates — KLAR (varianter saknas)](#fas-2-sparbara-grupptemplates--klar-varianter-saknas)
5. [Fas 3: Realtidsvisualisering (probes) — AVVAKTA](#fas-3-realtidsvisualisering-probes--avvakta)
6. [Fas 4: Pedagogiska verktyg — AVVAKTA](#fas-4-pedagogiska-verktyg--avvakta)
7. [Fas 5: Makroparametrar — AVVAKTA](#fas-5-makroparametrar--avvakta)
8. [Fas 6: Avancerade features — AVVAKTA](#fas-6-avancerade-features--avvakta)
9. [Enkel förbättring: Kollapsad grupp-content](#enkel-förbättring-kollapsad-grupp-content)
10. [Designbeslut](#designbeslut)
11. [Sammanfattning](#sammanfattning)

---

## 1. Vision

Modulgrupper löser tre problem samtidigt:

1. **Plottrighet** — Stora patchar med 10+ moduler blir oöverblickbara. Grupper kollapserar
   delmängder till hanterbara enheter.
2. **Återanvändning** — Vanliga byggblock (subtraktiv röst, effektkedja, FM-stack) kan sparas
   och återanvändas utan att byggas om varje gång.
3. **Pedagogik** — Genom att visa detaljerad realtidsinformation (vågformer, spektra, signalflöde)
   per grupp blir synten en interaktiv lärobok i ljudsyntes.

---

## 2. Arkitektur

Grupper implementeras som **UI-nivå metadata** ovanpå den befintliga platta `ModuleGraph`-arkitekturen.
Motorn ser fortfarande alla moduler platt — ingen nästlad grafprocessing, noll audio-overhead.

### Berörda crates (faktisk implementation)

| Crate | Ändring |
|-------|---------|
| `modular_synth` | `ModuleGroupState`, `GroupId`, `GroupTemplate` i `patch.rs`; grupp-UI i `patch_editor.rs`; template-browser i `dialogs.rs`; instansiering i `patch_bridge.rs`; template-filer i `group_templates/` |
| `synth_engine` | Ingen ändring |
| `synth_core` | Ingen ändring |
| `synth_modules` | Ingen ändring |
| `synth_dsp` | Ingen ändring |

---

## Fas 1: Visuell gruppering — KLAR

### Implementerad datamodell (`patch.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupId(pub u32);

pub type HexColor = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleGroupState {
    pub id: GroupId,
    pub name: String,
    pub color: Option<HexColor>,
    pub members: Vec<String>,          // ModuleId-strängar, inte typed ModuleId
    pub collapsed: bool,
    pub position: (f32, f32),
    pub exposed_inputs: Vec<ExposedPortState>,
    pub exposed_outputs: Vec<ExposedPortState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExposedPortState {
    pub label: String,
    pub module_id: String,             // Sträng, inte typed ModuleId
    pub port: String,
}
```

**Skillnader mot ursprunglig plan:**
- Använder `String` istället för typade `ModuleId`/`PortName` i serialiseringsformatet (konsekvent med resten av patch-formatet).
- Runtime-typen i `PatchEditor` heter `ModuleGroup` / `ExposedPort` (separata från serialiseringstypen `ModuleGroupState`).

### Implementerade features

- [x] `GroupId` och `ModuleGroupState` typer i `patch.rs`
- [x] `groups`-fält i `Patch` (bakåtkompatibelt: saknas = inga grupper)
- [x] Serialisering/deserialisering av grupper med positioner
- [x] Expanderad grupp: ram runt medlemsmoduler med gruppnamn i header
- [x] Kollapsad grupp: box med namn, exponerade portar (in/ut-kolumner), antal moduler
- [x] Skapa grupp från markerade moduler (högerklick-meny)
- [x] Expandera/kollapsa (dubbelklick eller knapp)
- [x] Ta bort grupp (raderar medlemsmoduler + kopplingar)
- [x] Avgruppera (behåller moduler + kopplingar)
- [x] Exponera/dölj portar manuellt
- [x] Auto-exponering vid gränskorsande kopplingar (`ensure_exposed_for_connection()`)
- [x] Kablar ritas till gruppens port-nod vid kollaps
- [x] Gruppnamn-redigering
- [x] Grupp-färgväljare
- [x] Grid-snapping av gruppposition
- [x] Kontextmeny per grupp (via menyknapp)

### Nyckel-filer

- `crates/modular_synth/src/patch.rs:81–116` — Datamodell
- `crates/modular_synth/src/gui/patch_editor.rs:2460–2649` — Kollapsad rendering
- `crates/modular_synth/src/gui/patch_editor.rs:2389–2458` — Expanderad rendering

---

## Fas 2: Sparbara grupptemplates — KLAR (varianter saknas)

### Implementerat template-format (`patch.rs`)

```rust
pub struct GroupTemplate {
    pub name: String,
    pub author: Option<String>,
    pub description: Option<String>,
    pub category: Option<GroupCategory>,
    pub tags: Vec<String>,
    pub color: Option<HexColor>,
    pub modules: Vec<ModuleState>,
    pub connections: Vec<ConnectionState>,
    pub exposed_inputs: Vec<ExposedPortState>,
    pub exposed_outputs: Vec<ExposedPortState>,
}

pub enum GroupCategory { Voice, Effect, Utility, Tutorial }
```

**Skillnader mot ursprunglig plan:**
- `category` är `Option<GroupCategory>` istället för required.
- Fälten `annotations: Vec<Annotation>` och `macro_params: Vec<MacroParam>` (planerade för fas 4/5) finns **inte** — läggs till om/när de faserna implementeras.
- Builder-metoder: `add_module()`, `add_connection()`, `expose_input()`, `expose_output()`.

### Implementerade features

- [x] `GroupTemplate`-format med serde-serialisering
- [x] ID-remapping vid instansiering (i `patch_bridge.rs`)
  - Allokerar nya `ModuleId` per instrument
  - Remappar alla kopplingar
  - Hanterar visualizers separat (Oscilloscope, LevelMeter, etc.)
  - Applicerar parametrar från template
- [x] Template-browser i GUI med kategori-filter och sök
- [x] 12 inbyggda templates (Voice: 3, Effect: 3, Utility: 4, Tutorial: 2)
- [x] Spara/ladda custom templates till disk (`~/.local/share/modular-synth/group-templates/`)
- [x] Spara existerande grupp som template (med metadata-dialog)
- [x] Ladda template in i existerande instrument (merge, inte ersätta)
- [x] `GroupTemplateManager` för disk-I/O och kategori-hantering

### Inte implementerat

- [ ] **Gruppvarianter** (parameter-presets per template) — Kvar som framtida feature

### Nyckel-filer

- `crates/modular_synth/src/group_templates/` — 12 template-filer
- `crates/modular_synth/src/io/group_template_manager.rs` — Filhantering
- `crates/modular_synth/src/gui/patch_bridge.rs:480–650` — Instansiering med ID-remapping
- `crates/modular_synth/src/gui/dialogs.rs:584–710` — Template-browser UI

---

## Fas 3: Realtidsvisualisering (probes) — AVVAKTA

> **Rekommendation: Avvakta.** Kräver engine-ändringar (ringbuffer per probe-punkt, villkorad
> datainsamling i `process_module()`). Hög komplexitet, medel värde i dagsläget. Standalone-
> visualizers (Oscilloscope, LevelMeter, SpectrumAnalyzer) täcker behovet utanför grupper.

### Kvar att göra

- [ ] Lock-free ringbuffer per probe-punkt
- [ ] Probe-datainsamling i `ModuleGraph::process_module()` (villkorad)
- [ ] `ProbeRenderer` i GUI (waveform, spektrum, meter)
- [ ] Signal-typ-anpassad rendering baserad på `PortType`
- [ ] Signalflödes-animation (tjocklek + ljusstyrka)
- [ ] Färgkodning av kopplingar per `PortType`

---

## Fas 4: Pedagogiska verktyg — AVVAKTA

> **Rekommendation: Avvakta.** De flesta features beror på fas 3 (probes). Solo/Mute per modul
> är oberoende men har lågt värde utan probes att jämföra i.

### Kvar att göra

- [ ] `BuildStep`-system för steg-för-steg-uppbyggnad
- [ ] Solo/Mute per modul i grupp-vy
- [ ] Annotations-rendering (info-ikoner, expanderbara texter)
- [ ] Parameterpåverkan-highlight vid hover
- [ ] Diff-vy mellan gruppvarianter
- [ ] Fler tutorial-templates

---

## Fas 5: Makroparametrar — AVVAKTA

> **Rekommendation: Avvakta.** Kraftfull feature men hög komplexitet. Kräver nytt dataflöde
> (makro → parameter-offset), GUI-editor, och integration med ModMatrix. Inget av detta finns
> idag. Bör vänta tills grupper används tillräckligt för att motivera investeringen.

### Kvar att göra

- [ ] `MacroParam` och `MacroMapping` typer
- [ ] Kurvberäkning (`MappingCurve`)
- [ ] Makro-rattar i kollapsad gruppvy
- [ ] Makro-editor i expanderad gruppvy
- [ ] MIDI CC-mappning
- [ ] ModMatrix-integration

---

## Fas 6: Avancerade features — AVVAKTA

> **Rekommendation: Avvakta tillsvidare.** Rekursiv nesting, polyfona grupper, freeze/snapshot —
> alla har mycket hög komplexitet. Implementera bara om en nivå visar sig otillräcklig.

---

## Enkel förbättring: Kollapsad grupp-content

> **Rekommendation: Kan implementeras som ett enkelt lyft.**

Just nu visar den kollapsade gruppen bara `"{N} modules"` i content-området (rad 2587–2601 i
`patch_editor.rs`). Enkel förbättring:

### Möjliga förbättringar (låg komplexitet)

1. **Lista medlemsmodulernas typ-namn** istället för bara antal. T.ex. "Osc, Filter, Env"
   i en kompakt lista — ger överblick utan att expandera.

2. **Visa kategori-ikon/badge** om gruppen skapades från en template med kategori
   (Voice/Effect/Utility/Tutorial).

3. **Visa template-description** (om den sparades från template) som tooltip eller kompakt text.

Dessa kräver ingen engine-ändring — enbart UI i `draw_collapsed_groups()`.

---

## Designbeslut

### Beslut 1: Platt graf med UI-metadata
Grupper är en vy-abstraktion. `ModuleGraph` förblir platt. Noll audio-overhead.

### Beslut 2: En nivå
En nivå täcker 90% av behoven. Rekursiv nesting kan läggas till senare.

### Beslut 3: Grupper inom befintlig voice_graph
Ingen ny voice allocation-logik. Polyfona grupper kan läggas till separat.

---

## Sammanfattning

| Fas | Feature | Status | Rekommendation |
|-----|---------|--------|----------------|
| 1 | Visuell gruppering | **KLAR** | — |
| 2 | Sparbara templates | **KLAR** (varianter saknas) | Varianter: avvakta |
| 3 | Realtidsprobes | Inte påbörjad | Avvakta (engine-ändringar krävs) |
| 4 | Pedagogiska verktyg | Inte påbörjad | Avvakta (beror på fas 3) |
| 5 | Makroparametrar | Inte påbörjad | Avvakta (hög komplexitet) |
| 6 | Avancerat | Inte påbörjad | Avvakta (mycket hög komplexitet) |
| — | Kollapsad grupp-content | Inte påbörjad | **Enkelt lyft, kan göras nu** |
