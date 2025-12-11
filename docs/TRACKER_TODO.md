# Tracker-import och Sequencer-vy

## Status: Grundfunktionalitet klar

**Mål:** Användaren kan importera en .MOD/.XM/.S3M-fil, se instrumenten i listan, se noterna i sequencern, stega mellan mönster och trycka på Play för att höra musiken.

---

## Avklarade uppgifter

### ✅ Steg 1: Uppdatera `import_song_file` i `src/gui/egui_backend.rs`
- [x] Rensa state innan import (instruments, instance_counters, visualization_buffers)
- [x] Skapa `InstrumentUiState` för varje sample
- [x] Populera `patch_editor` med SamplePlayer + StereoOutput + kablar
- [x] Synka med motor via `AddInstrument`, `AddModuleInstance`, `Connect`, `LoadSample`
- [x] Aktivera första instrumentet

### ✅ Steg 2: Pattern-visning (Data Sync)
- [x] Anropa `sync_grid_from_notes()` på alla mönster
- [x] Sätt `tracker_state.active_pattern` till giltigt ID

### ✅ Steg 3: Pattern-navigering i `src/gui/views/sequencer.rs`
- [x] Smart navigering för icke-sekventiella Pattern IDs
- [x] Sortera patterns och navigera i listan
- [x] Wrap-around vid början/slutet

### ✅ Steg 4: Säkerställ Uppspelning
- [x] Skicka `Stop` och `Rewind` efter import
- [x] Sätt BPM från importerad låt

### ✅ Steg 5: Song-uppspelning (Transport)
- [x] `SetSong` kommando för att skicka song till engine
- [x] Hantering av `Play`, `Stop`, `Pause`, `Rewind` i `synth_engine.rs`
- [x] Transport-knappar i sequencer-vyn kopplade till engine

### ✅ Steg 6: Sample Waveform-visning
- [x] `WaveformOverview` utökad med stereo-stöd (peaks_left, peaks_right)
- [x] Ny widget `draw_sample_waveform()` i `widgets/waveform_display.rs`
- [x] Waveform visas i SamplePlayer-moduler i patch_editor

---

## Förbättringar att göra

### Playback-position i waveform
- [ ] Koppla `playback_position` i `draw_sample_waveform()` till `VisualizationBuffer`
- [ ] SamplePlayer behöver skicka sin position till en buffer
- **Fil:** `src/modules/sample_player.rs`, `src/gui/patch_editor.rs`

### Song-sync mellan GUI och Engine
- [ ] Nuvarande implementation klonar song till engine - ändringar i GUI synkas inte
- [ ] Överväg delad `Arc<RwLock<Song>>` mellan GUI och engine
- [ ] Eller event-baserad synkning vid ändringar
- **Fil:** `src/gui/egui_backend.rs`, `src/engine/sequencer_engine.rs`

### Keyboard shortcuts för transport
- [ ] Space för Play/Stop toggle
- [ ] Home för Rewind
- [ ] Hantera i `handle_tracker_input()` eller global input handler
- **Fil:** `src/gui/views/sequencer.rs`

### Pattern-följning vid uppspelning
- [ ] Uppdatera `tracker_state.active_pattern` automatiskt när sequencer byter pattern
- [ ] Scrolla till aktuell rad under uppspelning
- **Fil:** `src/gui/views/sequencer.rs`, `src/engine/sequencer_engine.rs`

### Instrument-routing från sequencer
- [ ] Verifiera att `SeqInstrumentId` mappas korrekt till `InstrumentId`
- [ ] Samples skapas i ordning (0, 1, 2...) - bör matcha tracker-index
- **Fil:** `src/engine/sequencer_engine.rs`

---

## Tekniska detaljer

### Filer som ändrats för tracker-import:
- `src/gui/egui_backend.rs` - Import-logik, transport-routing
- `src/gui/views/sequencer.rs` - Pattern-navigering, transport-knappar
- `src/sequencer/view/state.rs` - `navigate_to_pattern()`
- `src/engine/synth_engine.rs` - Transport-kommandohantering
- `src/engine/commands.rs` - `SetSong` kommando
- `src/types/sample.rs` - Stereo `WaveformOverview`
- `src/gui/widgets/waveform_display.rs` - Ny widget
- `src/gui/patch_editor.rs` - Waveform-integration
- `src/gui/module_panel.rs` - `waveform_overview` i `ModulePanelState`