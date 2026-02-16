# Implementeringsplan: Modulgrupper

> Status: PLANERING | Datum: 2026-02-16 | Basversion: 0.134.0

## Innehåll

1. [Vision](#1-vision)
2. [Arkitektur](#2-arkitektur)
3. [Fas 1: Visuell gruppering](#fas-1-visuell-gruppering)
4. [Fas 2: Sparbara grupptemplates](#fas-2-sparbara-grupptemplates)
5. [Fas 3: Realtidsvisualisering (probes)](#fas-3-realtidsvisualisering-probes)
6. [Fas 4: Pedagogiska verktyg](#fas-4-pedagogiska-verktyg)
7. [Fas 5: Makroparametrar](#fas-5-makroparametrar)
8. [Fas 6: Avancerade features](#fas-6-avancerade-features)
9. [Designbeslut](#designbeslut)
10. [Sammanfattning](#sammanfattning)

---

## 1. Vision

Modulgrupper löser tre problem samtidigt:

1. **Plottrighet** — Stora patchar med 10+ moduler blir oöverblickbara. Grupper kollapserar
   delmängder till hanterbara enheter.
2. **Återanvändning** — Vanliga byggblock (subtraktiv röst, effektkedja, FM-stack) kan sparas
   och återanvändas utan att byggas om varje gång.
3. **Pedagogik** — Genom att visa detaljerad realtidsinformation (vågformer, spektra, signalflöde)
   per grupp blir synten en interaktiv lärobok i ljudsyntes.

### Vad som skiljer detta från "bara mappar"

| Egenskap | Visuell mapp | Modulgrupp |
|----------|-------------|------------|
| Kollapsbar i rack | Ja | Ja |
| Exponerade portar (in/ut) | Nej | Ja |
| Sparbar som template | Nej | Ja |
| Makroparametrar | Nej | Ja (fas 5) |
| Inline-probes | Nej | Ja (fas 3) |
| Steg-för-steg-läge | Nej | Ja (fas 4) |

---

## 2. Arkitektur

### 2.1 Övergripande design

Grupper implementeras som **UI-nivå metadata** ovanpå den befintliga platta `ModuleGraph`-arkitekturen.
Motorn ser fortfarande alla moduler platt — ingen nästlad grafprocessing.

```
┌─────────────────────────────────────────────────────┐
│  PatchEditor (GUI)                                  │
│  ┌────────────────┐  ┌────────────────────────────┐ │
│  │ Grupp-vy       │  │ Kollapsad grupp = en "box" │ │
│  │ (expanderad)   │  │ med exponerade portar       │ │
│  │ ┌────┐ ┌────┐  │  │                            │ │
│  │ │Osc │→│Filt│  │  │  [Pad Voice]               │ │
│  │ └────┘ └────┘  │  │   in:gate  out:L out:R     │ │
│  │    ↑           │  │                            │ │
│  │ ┌────┐         │  │                            │ │
│  │ │Env │         │  │                            │ │
│  │ └────┘         │  │                            │ │
│  └────────────────┘  └────────────────────────────┘ │
├─────────────────────────────────────────────────────┤
│  ModuleGraph (Engine) — platt, oförändrad           │
│  [osc-1] → [filter-1] → [output-1]                 │
│  [env-1] ↗                                          │
└─────────────────────────────────────────────────────┘
```

### 2.2 Varför platt graf (inte nästlad)

- Befintlig `ModuleGraph` fungerar utan ändringar
- Polyfoni hanteras exakt som idag (voice graph klonas per röst)
- Topologisk sortering behöver inte hantera rekursion
- Noll extra overhead i audio-tråden
- Grupper är i princip en "vy" ovanpå samma data

### 2.3 Berörda crates

| Crate | Ändring |
|-------|---------|
| `modular_synth` | Nytt: `ModuleGroup` i patch-format, grupp-vy i `PatchEditor`, template-system |
| `synth_engine` | Minimalt: probe-data-extraktion för visualisering |
| `synth_core` | Eventuellt: nya typer för `GroupId`, probe-metadata |
| `synth_modules` | Ingen ändring |
| `synth_dsp` | Ingen ändring |

---

## Fas 1: Visuell gruppering

*Mål: Reducera plottrighet genom att kunna gruppera moduler och kollapsa/expandera dem.*

### 1.1 Datamodell

```rust
/// Unikt ID för en grupp
pub struct GroupId(u32);

/// En grupp av moduler i patch-editorn
pub struct ModuleGroup {
    id: GroupId,
    name: String,
    color: Option<Color32>,          // Visuell färgkodning
    members: Vec<ModuleId>,          // Vilka moduler som ingår
    collapsed: bool,                 // Kollapsad = visas som en box
    position: (f32, f32),            // Position i rack (när kollapsad)
    exposed_inputs: Vec<ExposedPort>,
    exposed_outputs: Vec<ExposedPort>,
}

/// En port som är synlig när gruppen är kollapsad
pub struct ExposedPort {
    label: String,                   // Visningsnamn (t.ex. "Audio In")
    module_id: ModuleId,             // Intern modul
    port_name: PortName,             // Intern port
}
```

### 1.2 PatchEditor-ändringar

- Ny `groups: HashMap<GroupId, ModuleGroup>` i `PatchEditor`
- Rendering:
  - **Expanderad grupp:** Alla medlemsmoduler visas med en ram runt, gruppnamn i header
  - **Kollapsad grupp:** En enda box med gruppnamn och exponerade portar
- Interaktion:
  - Markera moduler → högerklick → "Skapa grupp"
  - Dubbelklicka kollapsad grupp → expandera
  - Dra modul in/ut ur grupp
  - Högerklicka port → "Exponera port" / "Dölj port"

### 1.3 Patch-serialisering

```json
{
  "name": "My Patch",
  "modules": [...],
  "connections": [...],
  "groups": [
    {
      "id": 1,
      "name": "Pad Voice",
      "color": "#4488aa",
      "members": ["osc-1", "filter-1", "env-1"],
      "collapsed": false,
      "position": [100.0, 200.0],
      "exposed_inputs": [
        { "label": "Gate", "module_id": "env-1", "port": "gate" }
      ],
      "exposed_outputs": [
        { "label": "Out", "module_id": "filter-1", "port": "out" }
      ]
    }
  ]
}
```

### 1.4 Uppgifter

- [ ] Lägg till `ModuleGroup` och `GroupId` typer
- [ ] Utöka `Patch`-formatet med `groups`-fält (bakåtkompatibelt: tomt = inga grupper)
- [ ] Implementera grupp-rendering i `PatchEditor` (expanderad/kollapsad)
- [ ] Implementera grupp-interaktion (skapa, expandera, kollapsa, exponera portar)
- [ ] Kopplingsritning mellan kollapsade grupper och fristående moduler

---

## Fas 2: Sparbara grupptemplates

*Mål: Spara och återanvänd grupper som byggblock i nya patchar.*

### 2.1 Template-format

```rust
pub struct GroupTemplate {
    name: String,
    author: Option<String>,
    description: Option<String>,
    category: GroupCategory,         // Voice, Effect, Utility, Tutorial
    tags: Vec<String>,
    modules: Vec<ModuleState>,       // Samma som i Patch
    connections: Vec<ConnectionState>,
    exposed_inputs: Vec<ExposedPortTemplate>,
    exposed_outputs: Vec<ExposedPortTemplate>,
    annotations: Vec<Annotation>,    // Förberedd för fas 4
    macro_params: Vec<MacroParam>,   // Förberedd för fas 5
}

pub enum GroupCategory {
    Voice,      // Oscillatorer + filter + envelopes
    Effect,     // Effektkedjor
    Utility,    // CV-verktyg, mixers
    Tutorial,   // Pedagogiska templates med annotationer
}
```

### 2.2 Instansiering

Vid insättning av en template i en patch:
1. Remappa alla `ModuleId` för att undvika konflikter (osc-1 → osc-3 om osc-1/osc-2 redan finns)
2. Skapa moduler via `PatchBridge::load_module()`
3. Skapa kopplingar med remappade ID:n
4. Skapa `ModuleGroup` med de nya modulerna

### 2.3 Gruppbibliotek

```
📁 Gruppbibliotek
├── 🎵 Röster (Voice)
│   ├── Classic Subtractive
│   ├── FM Bell
│   └── Wavetable Pad
├── 🎛️ Effekter (Effect)
│   ├── Stereo Chorus+Reverb
│   ├── Distortion Stack
│   └── Lo-fi Chain
├── 🔧 Utility
│   ├── Stereo Widener
│   └── Envelope Follower → CV
└── 📚 Tutorials
    ├── Subtraktiv syntes 101
    └── FM-syntes grunderna
```

Lagring: `~/.local/share/modular-synth/group-templates/`

### 2.4 Gruppvarianter (presets)

Samma gruppstruktur (moduler + kopplingar) med olika parametervärden:

```json
{
  "template": "Classic Subtractive",
  "variants": [
    {
      "name": "Soft Pad",
      "parameters": { "filter-1:cutoff": 400.0, "env-1:attack": 0.8 }
    },
    {
      "name": "Pluck Bass",
      "parameters": { "filter-1:cutoff": 2000.0, "env-1:attack": 0.005 }
    }
  ]
}
```

### 2.5 Uppgifter

- [ ] Definiera `GroupTemplate`-format med serialisering
- [ ] Implementera ID-remapping vid instansiering
- [ ] Bygg grupp-browser i GUI (lista, sök, kategorier)
- [ ] Skapa 5-10 inbyggda grupptemplates
- [ ] Spara/ladda custom templates
- [ ] Implementera gruppvarianter (parameter-presets per template)

---

## Fas 3: Realtidsvisualisering (probes)

*Mål: Visa vågformer, spektra och signaldata direkt på kopplingar inom en expanderad grupp.*

### 3.1 Inline-probes på kopplingar

Varje koppling inom en expanderad grupp kan visa en liten realtidsvisning:

```
┌──────┐    ┌~waveform~┐    ┌──────┐
│ Osc  │━━━━┤ /\/\/\   ├━━━━│Filter│
│      │    └~~~~~~~~~~┘    │      │
└──────┘                    └──────┘
```

**Tre visningslägen per probe:**

| Läge | Visar | Bäst för |
|------|-------|----------|
| Waveform | Oscilloskop-vy (tid vs amplitud) | Audio-signaler, envelopes |
| Spektrum | FFT-vy (frekvens vs amplitud) | Före/efter filter, distortion |
| Meter | Peak/RMS-stapel + dB-värde | Snabb nivåkoll |

### 3.2 Signal-typ-anpassad rendering

Probes detekterar automatiskt signaltyp och anpassar visningen:

| Signaltyp | Detektion | Visning |
|-----------|-----------|---------|
| Audio (snabb) | Frekvens > 20 Hz | Waveform + spektrum |
| CV/Modulation (långsam) | Frekvens < 20 Hz | Kurva över tid + aktuellt värde i siffror |
| Gate (on/off) | Binärt 0/1-mönster | Pulsindikator med on/off-status |
| Trigger (kort puls) | Korta transienter | Blinkande punkt vid trigger |

### 3.3 Dataflöde (realtidssäkert)

```
Audio-tråd                          GUI-tråd
┌──────────┐                        ┌──────────────┐
│ process() │  ring buffer (lock-free)  │ ProbeRenderer│
│ → skriver ├──────────────────────→│ → läser      │
│   samples │  (senaste N samples)  │   och ritar   │
└──────────┘                        └──────────────┘
```

- Audio-tråden skriver till en lock-free ringbuffer per aktiv probe
- GUI-tråden läser och renderar i sin egen takt (60 fps)
- Probes aktiveras **bara** för expanderade gruppers kopplingar (ingen overhead för kollapsade)
- Befintlig infrastruktur: `Oscilloscope`-modulen använder redan liknande mekanism

### 3.4 Port-info-vy

Varje port i grupp-vyn kan visa en kompakt info-panel:

```
┌─ filter-1: in ─────────┐
│  Peak: -6.2 dB          │
│  RMS:  -12.4 dB         │
│  Freq: 440 Hz (A4)      │
│  ┌──────────────────┐   │
│  │ ╱╲╱╲╱╲╱╲╱╲╱╲╱╲  │   │
│  └──────────────────┘   │
└─────────────────────────┘
```

### 3.5 Signalflödes-animation

Kopplingar i grupp-vyn animeras baserat på signalnivå:

- **Tjocklek:** Proportionell mot RMS-nivå
- **Ljusstyrka/färg:** Starkare signal = ljusare
- **Vid tystnad:** Tunna, mörka linjer
- **Vid note-on:** Kopplingarna "tänds" i processingsordning

### 3.6 Färgkodning per signaltyp

| Typ | Färg | Motivering |
|-----|------|------------|
| Audio | Blå/cyan | Associeras med oscilloskop |
| CV/Modulation | Grön | Vanlig konvention i modulärsyntar |
| Gate | Gul/orange | "Varning" — binärt, styrande |
| Trigger | Röd | Kort, snabb — uppmärksamhet |

Kopplingar i grupp-vyn färgas automatiskt baserat på `PortType`.

### 3.7 Uppgifter

- [ ] Implementera lock-free ringbuffer per probe-punkt (eller återanvänd befintlig)
- [ ] Lägg till probe-datainsamling i `ModuleGraph::process_module()` (villkorad, bara aktiva probes)
- [ ] Implementera `ProbeRenderer` i GUI (waveform, spektrum, meter)
- [ ] Auto-detektion av signaltyp (audio/CV/gate/trigger)
- [ ] Signalflödes-animation (tjocklek + ljusstyrka baserat på nivå)
- [ ] Färgkodning av kopplingar baserat på `PortType`
- [ ] Port-info-panel (peak, RMS, grundfrekvens)

---

## Fas 4: Pedagogiska verktyg

*Mål: Göra grupper till interaktiva läromedel för ljudsyntes.*

### 4.1 "Bygg steg för steg"-läge

En grupptemplate kan ha en definierad ordning där moduler introduceras en i taget:

```rust
pub struct BuildStep {
    step: u8,
    add_modules: Vec<String>,        // Moduler att visa i detta steg
    add_connections: Vec<(String, String)>, // Kopplingar att aktivera
    title: String,                   // "Steg 2: Lägg till filter"
    description: String,             // "Filtret tar bort övertoner..."
    highlight: Vec<String>,          // Moduler/kopplingar att markera
}
```

**Flöde:**
1. Steg 1: Visa bara oscillatorn → hör rå sågvåg, se vågformen
2. Steg 2: Lägg till filter → hör skillnaden, se hur spektrumet ändras
3. Steg 3: Lägg till envelope → hör hur klangen formas över tid
4. Steg 4: Lägg till LFO-modulation → hör rörelsen, se CV-signalen

Inline-probes (fas 3) aktiva under hela processen gör varje steg visuellt.

### 4.2 Solo/Mute per modul

Inom en expanderad grupp:

- **Mute:** Bypassa en modul → hör vad den bidrog med
- **Solo:** Hör bara utgången från en specifik modul isolerad
- **A/B-växling:** Snabb toggle för att jämföra med/utan en modul

Befintlig bypass-infrastruktur i `ModuleGraph` kan återanvändas.

### 4.3 Annotationer

Grupptemplates kan inkludera förklarande text bunden till moduler och kopplingar:

```rust
pub struct Annotation {
    target: AnnotationTarget,
    text: String,
    detail: Option<String>,          // Expanderbar förklaring
}

pub enum AnnotationTarget {
    Module(String),                  // "osc-1"
    Connection(String, String),      // "osc-1:out" → "filter-1:in"
    Port(String, String),            // "filter-1", "cutoff"
    Group,                           // Hela gruppen
}
```

Visas som info-ikoner i grupp-vyn. Klick expanderar förklaringen.

### 4.4 Parameterpåverkan-visualisering

När användaren hovrar över en ratt:

1. Markera alla kopplingar och moduler som påverkas
2. Visa vilka modulationskällor som styr samma parameter (ModMatrix, LFO)
3. Rita en tillfällig "påverkanskedja" genom gruppen

### 4.5 Diff-vy mellan gruppvarianter

Visa skillnader mellan två varianter av samma grupptemplate:

```
┌─ Soft Pad vs Pluck Bass ──────────────────────┐
│  filter-1:cutoff    400 Hz  →  2000 Hz  (+5x) │
│  env-1:attack       800 ms  →  5 ms     (-99%) │
│  env-1:release      2.0 s   →  0.3 s    (-85%) │
│  osc-1:detune       0.15    →  0.0      (av)   │
└────────────────────────────────────────────────┘
```

Pedagogiskt: visar exakt vilka parametrar som skiljer en mjuk pad från en knäppig bas.

### 4.6 Uppgifter

- [ ] Implementera `BuildStep`-system för steg-för-steg-uppbyggnad
- [ ] Lägg till Solo/Mute-knappar per modul i grupp-vy
- [ ] Implementera annotations-rendering (info-ikoner, expanderbara texter)
- [ ] Parameterpåverkan-highlight vid hover
- [ ] Diff-vy mellan gruppvarianter
- [ ] Skapa 3-5 tutorial-templates ("Subtraktiv 101", "FM-syntes", "Effektkedja")

---

## Fas 5: Makroparametrar

*Mål: Grupper exponerar egna rattar som styr flera interna parametrar samtidigt.*

### 5.1 Datamodell

```rust
pub struct MacroParam {
    name: String,                    // "Brightness"
    range: (f32, f32),               // (0.0, 1.0)
    default: f32,                    // 0.5
    mappings: Vec<MacroMapping>,
}

pub struct MacroMapping {
    target_module: String,           // "filter-1"
    target_param: String,            // "cutoff"
    source_range: (f32, f32),        // Makro-input range
    target_range: (f32, f32),        // Parameter output range
    curve: MappingCurve,             // Linjär, exponentiell, etc.
}

pub enum MappingCurve {
    Linear,
    Exponential(f32),                // Exponent
    Logarithmic,
    SCurve,
    Inverted(Box<MappingCurve>),     // Vänd riktning
}
```

### 5.2 Användargränssnitt

I kollapsad gruppvy visas makroparametrar som rattar:

```
┌─ Pad Voice ─────────────────────────────┐
│  [Brightness] [Warmth] [Movement]       │
│      0.7        0.4       0.6           │
│                                         │
│  in:gate  in:pitch   out:L  out:R       │
└─────────────────────────────────────────┘
```

### 5.3 MIDI-mappning

Makroparametrar kan mappas till MIDI CC:

```rust
pub struct MacroMidiMapping {
    macro_index: usize,
    cc_number: u8,
}
```

En makro = en CC-ratt på en MIDI-kontroller. Enklare än att mappa individuella parametrar.

### 5.4 Relation till befintlig ModMatrix

`ModMatrix` hanterar per-röst-modulation (LFO → filter). Makroparametrar är annorlunda:

- ModMatrix: realtidsmodulation per sample
- Makro: parameter-offsetting, som att vrida en ratt

Makros kan dock vara *källor* i ModMatrix, vilket ger indirekt realtidsmodulation.

### 5.5 Uppgifter

- [ ] Implementera `MacroParam` och `MacroMapping`
- [ ] Implementera kurvberäkning (`MappingCurve`)
- [ ] Rendera makro-rattar i kollapsad gruppvy
- [ ] Makro-editor i expanderad gruppvy (dra mappning från makro till parameter)
- [ ] MIDI CC-mappning för makros
- [ ] Integration med ModMatrix (makro som modulationskälla)

---

## Fas 6: Avancerade features

*Valfria utökningar som kan implementeras om behov uppstår.*

### 6.1 Freeze/Snapshot

Frys signalen vid ett specifikt ögonblick och undersök alla signaler statiskt:

- Pausa vid note-on och stega framåt sample för sample
- Alla probes visar frysta värden
- Extremt pedagogiskt för att förstå transienter och envelope-timing

### 6.2 Rekursiv nesting (grupper i grupper)

Tillåt att en grupp innehåller andra grupper. Kräver:

- Hierarkisk navigation (breadcrumbs: "Patch > Pad Voice > Filter Section")
- Port-exponering genom flera nivåer
- ID-namespacing för att undvika konflikter

Rekommendation: Implementera bara om en nivå visar sig otillräcklig i praktiken.

### 6.3 Polyfona grupper (egen voice allocator)

En grupp som hanterar egen polyfoni — i princip ett instrument-i-instrument:

- Egen `VoiceAllocator`
- Propagering av `note_on`/`note_off`
- Egen effektkedja (valfritt)

Mycket hög komplexitet. Troligen bara relevant för multi-timbral design.

### 6.4 Grupp-import/export

- Exportera grupp som fristående fil för delning
- Importera andras grupptemplates
- Potentiellt: online-bibliotek med community-templates

---

## Designbeslut

### Beslut 1: Platt graf vs nästlad graf

**Valt: Platt graf med UI-metadata**

Grupper är en vy-abstraktion, inte en engine-abstraktion. `ModuleGraph` förblir platt.

*Motivering:*
- Polyfoni fungerar utan ändringar
- Noll audio-overhead
- Enklare att implementera steg för steg
- Kan alltid utökas till nästlad graf senare om det behövs

### Beslut 2: En nivå vs rekursiv nesting

**Valt: En nivå (fas 1-5), rekursiv valfri (fas 6)**

*Motivering:*
- En nivå täcker 90% av behoven
- Drastiskt enklare UI
- Bakåtkompatibelt om nesting läggs till senare

### Beslut 3: Grupper inom röst (inte som egen röst)

**Valt: Grupper inom befintlig voice_graph**

*Motivering:*
- Enklaste modellen
- Ingen ny voice allocation-logik
- Polyfona grupper kan läggas till som separat feature (fas 6)

---

## Sammanfattning

| Fas | Feature | Värde | Komplexitet | Beroenden |
|-----|---------|-------|-------------|-----------|
| 1 | Visuell gruppering | Löser plottrighet | Låg-Medel | Ingen |
| 2 | Sparbara templates | Återanvändning | Medel | Fas 1 |
| 3 | Realtidsprobes | Visualisering, pedagogik | Medel-Hög | Fas 1 |
| 4 | Pedagogiska verktyg | Interaktiv lärobok | Medel | Fas 1, 3 |
| 5 | Makroparametrar | Kraftfulla byggblock | Medel-Hög | Fas 1, 2 |
| 6 | Avancerat (nesting, polyfoni) | Maximal flexibilitet | Hög-Mycket hög | Fas 1-5 |

Varje fas bygger på föregående utan att kräva omskrivning. Fas 1 ger omedelbart värde
och kan implementeras utan engine-ändringar. Fas 3 (probes) är den mest unikt värdefulla
featuren — den förvandlar grupper från organisationsverktyg till pedagogiska instrument.
