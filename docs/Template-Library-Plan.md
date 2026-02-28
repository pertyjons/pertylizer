# Implementeringsplan: Templatebibliotek (grupper + patchar)

> Status: PLANERING | Datum: 2026-02-28 | Basversion: 0.192.0

## 1. Vision

Ett templatebibliotek ska göra det enkelt att återanvända bra byggblock och patchar:

- **Snabbt skapande**: Återanvänd färdiga röster, effekter och utilities utan att bygga om varje gång.
- **Kvalitet**: Tydliga default‑värden och tydlig exponering av portar.
- **Delning**: Ett enkelt format som kan delas som vanliga JSON‑filer.

## 2. Omfattning (MVP)

- **Två typer**: Grupp‑templates (`GroupTemplate`) och Patch‑templates (patch‑JSON).
- **En library‑vy**: Ingen uppdelning mellan curated och user i UI.
- **Metadata per fil**: Namn, kategori, taggar, beskrivning, author, license, min_version.
- **Ladda + spara**: Både för grupp‑ och patch‑templates.

## 3. Format & struktur

### 3.1 Kataloger

- Grupp‑templates: `~/.local/share/modular-synth/group-templates` (redan)
- Patch‑templates: `~/.local/share/modular-synth/patch-templates` (ny)
- Om vi skeppar built‑ins: de ligger i `assets/` men visas i samma lista (ingen särskild flik).

### 3.2 Metadata i varje fil

**GroupTemplate‑JSON** (utökas vid behov):

- `name`, `author`, `description`, `category`, `tags` finns redan.
- Lägg till `license` och `min_app_version` som valfria fält.

**Patch‑template‑JSON** (existerande patch‑format):

- Använder redan `name`, `author`, `description`, `tags`.
- Vid behov kan `template_meta` läggas till senare, men MVP håller sig till befintliga fält.

## 4. UX & flöden

- **Template‑browser** med två typer: `Group` och `Patch` (tabbar eller filter).
- **Filter**: sök, kategori, taggar.
- **Actions**:
  - `Insert` för grupp‑template.
  - `Open` för patch‑template (laddar patchen).
  - `Save as Template` för grupp (via GruppModuler‑menyn).
  - `Save Patch as Template` i File‑menyn.
- **File‑picker**: möjligt att ladda template från valfri JSON‑fil.

## 5. Faser

### Fas 1 — Grupp‑templates (library‑vy)

- Samla alla group‑templates från template‑katalogen (och ev. assets) i en lista.
- Visa dem i samma browser utan curated/user‑split.
- Spara template från GruppModuler‑menyn.
- Ladda via browser eller file‑picker.

### Fas 2 — Patch‑templates

- Ny patch‑template browser (eller sektion i File‑menyn).
- `Save Patch as Template` som sparar till patch‑template‑katalogen.
- Ladda patch‑template till aktivt instrument.

### Fas 3 — Metadata‑polish

- Stöd för `license` och `min_app_version` i GroupTemplate.
- Validera metadata (tomt namn, ogiltiga tags) vid save.

## 6. Acceptanskriterier (Fas 1)

- Alla grupp‑templates i template‑katalogen syns i browsern.
- Browsern kan filtrera på söktext och kategori.
- `Save as Template` i GruppModuler‑menyn skapar en fil med metadata i samma JSON.
- `Load` fungerar både via browser och file‑picker.

## 7. Risker & beslut

- **Filnamnskrock**: spara med säkert filnamn och suffix om fil redan finns.
- **Versionering**: `min_app_version` är valfri; okända fält ignoreras.
