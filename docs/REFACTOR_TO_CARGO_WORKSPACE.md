# TODO: Refaktorering till Cargo Workspace

Denna refaktorering syftar till att dela upp `modular-synth` i flera mindre crates för att förbättra kompileringstider och arkitektur.

### Fas 1: Förberedelser & Workspace

- [ ] Skapa mappen `crates/` i roten av projektet.
- [ ] Säkerhetskopiera nuvarande `Cargo.toml`.
- [ ] Skapa en ny `Cargo.toml` i roten med följande workspace-konfiguration:
  ```toml
  [workspace]
  resolver = "2"
  members = [
      "crates/synth_core",
      "crates/synth_dsp",
      "crates/synth_sequencer",
      "crates/synth_modules",
      "crates/synth_engine",
      "crates/modular_synth",
  ]

  [workspace.dependencies]
  serde = { version = "1.0", features = ["derive"] }
  parking_lot = "0.12"
  # Lägg till andra gemensamma versioner här
  ```

### Fas 2: Skapa `synth_core` (Grundplattan)

*Inga interna beroenden. Här bor gemensamma typer och traits.*

- [ ] Skapa mapp: `crates/synth_core/src/`.
- [ ] Skapa `crates/synth_core/Cargo.toml` (beroenden: `serde`, `parking_lot`).
- [ ] Flytta `src/types/` → `crates/synth_core/src/types/`.
- [ ] Flytta `src/engine/params/` → `crates/synth_core/src/params/`. **(Viktigt\!)**
- [ ] Flytta `src/engine/commands.rs` → `crates/synth_core/src/commands.rs`.
- [ ] Flytta `src/modules/core.rs` → `crates/synth_core/src/traits.rs` (döp om filen).
- [ ] Flytta `src/audio/traits.rs` → `crates/synth_core/src/audio_traits.rs`.
- [ ] Skapa `crates/synth_core/src/lib.rs` och exponera alla moduler (`pub mod`).

### Fas 3: Skapa `synth_dsp` (Matematik)

*Beroende: `synth_core`.*

- [ ] Skapa mapp: `crates/synth_dsp/src/`.
- [ ] Skapa `crates/synth_dsp/Cargo.toml`.
- [ ] Flytta `src/dsp/` → `crates/synth_dsp/src/dsp/`.
- [ ] Uppdatera imports i `synth_dsp` att använda typer från `synth_core`.

### Fas 4: Skapa `synth_sequencer` (Musiklogik)

*Beroende: `synth_core`.*

- [ ] Skapa mapp: `crates/synth_sequencer/src/`.
- [ ] Skapa `crates/synth_sequencer/Cargo.toml`.
- [ ] Flytta `src/sequencer/` → `crates/synth_sequencer/src/sequencer/`.
- [ ] Uppdatera imports att använda `synth_core`.

### Fas 5: Skapa `synth_modules` (Byggstenar)

*Beroende: `synth_core`, `synth_dsp`.*

- [ ] Skapa mapp: `crates/synth_modules/src/`.
- [ ] Skapa `crates/synth_modules/Cargo.toml`.
- [ ] Flytta `src/modules/` (förutom `core.rs`) → `crates/synth_modules/src/modules/`.
- [ ] Flytta `src/effects/` → `crates/synth_modules/src/effects/`.
- [ ] Refaktorera: Ändra `use crate::modules::core::PolyModule` till `use synth_core::traits::PolyModule`.
- [ ] Refaktorera: Ändra parameter-imports till `synth_core::params`.

### Fas 6: Skapa `synth_engine` (Logikmotorn)

*Beroende: `synth_core`, `synth_modules`, `synth_sequencer`.*

- [ ] Skapa mapp: `crates/synth_engine/src/`.
- [ ] Skapa `crates/synth_engine/Cargo.toml`.
- [ ] Flytta resterande filer från `src/engine/` → `crates/synth_engine/src/engine/`.
- [ ] **OBS:** Flytta INTE `src/audio/backends/` hit (se nästa fas).
- [ ] Se till att motorn implementerar `AudioProcessor` från `synth_core` men inte känner till `cpal`.

### Fas 7: Skapa `modular_synth` (Applikationen & GUI)

*Beroende: Alla ovanstående + `cpal`, `egui`, `midir`.*

- [ ] Skapa mapp: `crates/modular_synth/src/`.
- [ ] Skapa `crates/modular_synth/Cargo.toml`.
- [ ] Flytta `src/main.rs`, `src/gui/`, `src/io/` till denna crate.
- [ ] Flytta `src/audio/` (inklusive `backends/cpal_backend.rs`) hit.
- [ ] Uppdatera `src/main.rs` så den startar `cpal` och skapar en `SynthEngine` via traits.

### Fas 8: Städning & Verifiering

- [ ] Ta bort den gamla `src/`-mappen i roten.
- [ ] Kör `cargo check` i roten (ska verifiera hela workspacen).
- [ ] Kör `cargo test`.
- [ ] Kör `cargo run` och verifiera att applikationen startar och låter som förut.