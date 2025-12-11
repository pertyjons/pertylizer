# Version History

## [0.46.0] - 2025
### Added - Tracker Effects Implementation

Fullständig implementation av tracker-effekter för MOD/XM/S3M-import.

#### Effekter Implementerade
- **Volume Effects:** SetVolume, VolumeSlide, FineVolumeSlide, Tremolo
- **Pitch Effects:** Arpeggio, PortamentoUp, PortamentoDown, TonePortamento, Vibrato
- **Panning Effects:** SetPanning, PanningSlide
- **Timing Effects:** NoteDelay, NoteCut, NoteFadeOut, Retrigger
- **Global Effects:** SetTempo, SetSpeed, PatternBreak, PatternJump, PatternLoop, PatternDelay
- **Miscellaneous:** SampleOffset, FineTune, Glissando, VibratoWaveform, TremoloWaveform

#### Nya Typer (typsäkra wrappers)
- **`TrackerSpeed`** - Ticks per rad (default: 6)
- **`TickInRow`** - Position inom en rad
- **`TrackerSampleOffset`** - Startposition för sample-uppspelning
- **`PitchCents`** - Tonhöjdsförskjutning i cents
- **`PortamentoDirection`** - Up/Down/Off för portamento

#### Arkitektur
- **`ChannelEffectProcessor`** - Hanterar alla kanal-effekter
- **`ChannelEffectState`** - Per-kanal-tillstånd (volym, panning, vibrato-fas, etc.)
- **`ChannelModulation`** - Modulationsvärden som appliceras på noter
- **`GlobalCommand`** - Kommandon som påverkar sequencer-nivån

#### Effektkonvertering (xmrs → vår modell)
- `TrackEffect` → `EffectCommand` (not-nivå effekter)
- `GlobalEffect` → `EffectCommand` (pattern-nivå effekter)
- Waveform-mapping för vibrato/tremolo

#### Analysverktyg
- **`analyze_all_trackers`** - Analyserar alla tracker-filer i en katalog
  - Rapporterar vilka effekter som används
  - Identifierar saknade/ostödda funktioner
  - Listar envelope- och sample-features

```bash
cargo run --example analyze_all_trackers -- /path/to/music
```

#### Filer som ändrats
- `src/engine/tracker_effects.rs` - **NY** - Effektprocessing
- `src/engine/mod.rs` - Exporterar tracker_effects
- `src/engine/sequencer_engine.rs` - Integrerar effektprocessing
- `src/io/import/tracker.rs` - Konverterar xmrs-effekter till EffectCommand
- `examples/analyze_all_trackers.rs` - **NY** - Analysverktyg

---

## [0.45.0] - 2025
### Fixed - Tracker Import & Sequencer Playback

Kritiska fixar för tyst uppspelning av importerade tracker-moduler.

#### Bugfixar
- **Tyst XM-uppspelning:** `focused_instrument` rensas nu efter import så att alla instrument spelas i Sequencer-vyn
- **Fokuserad routing:** Sequencer-händelser respekterar nu `focused_instrument` för solo-läge i Rack-vyn

#### Nytt verktyg
- **`analyze_tracker`** - Diagnostikverktyg för MOD/XM/S3M-filer
  - Visar modulinfo (namn, BPM, tempo, kanaler, mönster)
  - Instrumentdetaljer (samples, volym-envelopes, pan, fadeout)
  - Sample-information (datalängd, volym, loop-inställningar)
  - Mönsterordning och noter i första mönstret
  - Sammanfattning med statistik

```bash
cargo run --example analyze_tracker -- /path/to/file.mod
cargo run --example analyze_tracker -- /path/to/file.xm
cargo run --example analyze_tracker -- /path/to/file.s3m
```

#### Filer som ändrats
- `src/gui/egui_backend.rs` - Rensar `focused_instrument` efter import till Sequencer-vy
- `src/engine/synth_engine.rs` - Kollapsade if-satser (clippy-fix)
- `examples/analyze_tracker.rs` - Nytt verktyg (ersätter analyze_xm.rs)

---

## [0.44.0] - 2025
### Added - Native File Dialogs

Ersätter manuella fildialog med `egui-file-dialog` crate för native filbläddrare.

#### File Dialog Integration
- **`egui-file-dialog` v0.12** - Native filväljare med filterstöd
- **Öppna patch** - Ny menypost "Open Patch..." för att öppna sparade patches
- **Spara patch** - "Save Patch..." öppnar filväljare med .json-filter
- **Import song** - "Import Song..." med filter för .mod/.xm/.s3m
- **Load Built-in** - Behållen för inbyggda example-patches

#### API-ändringar
- **`FileDialogMode`** enum: `OpenPatch`, `SavePatch`, `ImportSong`, `OpenSample`
- **`FileDialogResult`** enum: `Picked(PathBuf, mode)`, `Saved(PathBuf, mode)`
- **`DialogState`** metoder:
  - `open_open_patch_dialog()` - Öppna patch-fil
  - `open_save_patch_dialog(default_name)` - Spara patch
  - `open_import_song_dialog()` - Import tracker-fil
  - `open_sample_dialog()` - Öppna WAV-sample (förberett för framtida bruk)
  - `update_file_dialog(ctx)` - Hanterar dialog-state och returnerar resultat

#### Filer som ändrats
- `Cargo.toml` - `egui-file-dialog = "0.12"` tillagt
- `src/gui/dialogs.rs` - FileDialog integration, nya typer och metoder
- `src/gui/egui_backend.rs` - Uppdaterade menyer och FileDialogResult-hantering

---

## [0.43.0] - 2025
### Added - Focused Instrument & Tracker Playback Fixes

Lösningar för MIDI-kanalkonflikter och korrekt tracker-uppspelning.

#### Focused Instrument (Keyboard Routing)
- **Problem:** Med >16 instrument delade flera samma MIDI-kanal, alla spelade samtidigt
- **Lösning:** "Focused Instrument" - keyboard input (kanal 0) går endast till valt instrument
- **`EngineState`:** `focused_instrument: AtomicU32` för trådsäker state
- **`EngineCommand::SetFocusedInstrument`** - Nytt kommando för GUI-styrning
- **`EngineHandle`:** `set_focused_instrument()`, `get_focused_instrument()` metoder
- **GUI-integration:** Automatiskt fokus vid instrumentval, import, och borttagning

#### Mono-per-Track (Tracker-beteende)
- **Problem:** Loopade ljud försvann inte vid nya noter på samma track
- **Lösning:** `TrackId` newtype för typsäker track-identifiering
- **`Note::track`:** `Option<TrackId>` för mono-per-track routing
- **Sequencer:** `stop_notes_on_track()` stänger av föregående not automatiskt
- **Tracker-import:** Sätter `TrackId` för varje kanal vid import

#### Sample Player Fixes
- **Release Mode:** Non-looped samples använder nu `ReleaseMode::PlayToEnd` (default)
  - Drums/one-shots spelar klart istället för abrupt stopp
  - Looped samples behåller `ReleaseMode::Immediate`
- **Interpolation:** Konfigurerbar i GUI med Cubic som default
  - Nearest, Linear, Cubic, Hermite, Lagrange, Sinc8, Sinc16

#### Pattern View Improvements
- **Follow Playback:** Pattern-vyn följer nu aktiv rad under uppspelning
- **Auto-switch Pattern:** Automatiskt byte till aktivt pattern
- **Dynamiska tracks:** Visar rätt antal tracks från importerad fil (inte hårdkodat 4)
- **Pattern display:** Visar "1/3" format (aktuellt/totalt) istället för hex

#### Filer som ändrats
- `src/engine/state.rs` - `focused_instrument`, `NO_FOCUSED_INSTRUMENT`
- `src/engine/commands.rs` - `SetFocusedInstrument` kommando
- `src/engine/synth_engine.rs` - note routing, handle methods
- `src/sequencer/note.rs` - `track: Option<TrackId>`, `with_track()`
- `src/sequencer/ids.rs` - `TrackId` exporteras
- `src/engine/sequencer_engine.rs` - `ActiveNote::track`, `stop_notes_on_track()`
- `src/io/import/tracker.rs` - Sätter `TrackId` vid import
- `src/modules/sample_player.rs` - Smart `ReleaseMode` i `load_sample()`
- `src/gui/instrument_rack.rs` - Focused instrument vid val/borttagning
- `src/gui/egui_backend.rs` - Focused instrument vid start/import

---

## [0.42.0] - 2025
### Added - Song Playback & Sample Waveform Display

Uppspelning av importerade tracker-filer och visualisering av samples.

#### Transport-kontroll
- **`SetSong` kommando** - Skickar song till `SequencerEngine` för uppspelning
- **Transport-kommandon** i `synth_engine.rs`: `Play`, `Stop`, `Pause`, `Rewind`
- **GUI-knappar** i sequencer-vyn kopplade till engine

#### Sample Waveform-visning
- **`WaveformOverview`** utökad med stereo-stöd:
  - `peaks_left` / `peaks_right` för separata kanaler
  - `is_stereo` flagga
  - Nya metoder `peak_left_at()`, `peak_right_at()`
- **Ny widget** `draw_sample_waveform()` i `widgets/waveform_display.rs`:
  - Mono: Enkel centrerad vågform
  - Stereo: L/R separerade (ovanför/under mittlinje)
  - Stöd för playback-position indikator
- **Integration** i `patch_editor.rs` - Waveform visas automatiskt i SamplePlayer-moduler

#### Filer som ändrats
- `src/engine/commands.rs` - `SetSong` kommando
- `src/engine/synth_engine.rs` - Transport-hantering
- `src/engine/hub.rs`, `src/engine/transactions.rs` - SetSong stöd
- `src/gui/views/sequencer.rs` - `TransportAction` enum, knappkoppling
- `src/gui/egui_backend.rs` - Transport-routing, SetSong vid import
- `src/types/sample.rs` - Stereo `WaveformOverview`
- `src/gui/widgets/waveform_display.rs` - Ny widget
- `src/gui/patch_editor.rs` - Waveform i SamplePlayer
- `src/gui/module_panel.rs` - `waveform_overview` i `ModulePanelState`

---

## [0.41.0] - 2025
### Added - Tracker Import (MOD/XM/S3M)

Import av klassiska tracker-filer direkt i synthen.

#### Import-arkitektur (`src/io/import/`)

- **`SongImporter` trait** - Utökningsbar arkitektur för filformat:
  - `name()` - Importernamn
  - `extensions()` - Stödda filändelser
  - `can_import()` - Formatdetektering
  - `import()` - Returnerar `ImportResult<ImportedSong>`

- **`ImportedSong`** - Resultat med:
  - `song: Song` - Konverterade patterns och arrangement
  - `samples: Vec<Arc<Sample>>` - Extraherade samples

- **`ImportError`** enum - Typade fel (NotFound, Io, UnsupportedFormat, Parse, InvalidData)

#### Tracker-loader (`src/io/import/tracker.rs`)

- **`TrackerImporter`** - Stöd för MOD, XM, S3M via `xmrs` crate
- **Sample-konvertering**:
  - 8-bit: `i8 / 128.0` → `SampleValue`
  - 16-bit: `i16 / 32768.0` → `SampleValue`
  - Stereo och float-format stöds
- **Pattern-konvertering**:
  - xmrs `TrackUnit` → interna `Note`
  - Beräknar ticks per rad baserat på tracker-speed
  - Hanterar `Pitch::None` och `Pitch::Off` (keyoff)
  - Sätter `RowResolution` för korrekt tracker-visning
- **Tempo-hantering**: Använder modulens `default_bpm` och `default_tempo`

#### GUI-integration

- **File → Import Song...** menyval
- **Import Song-dialog** (`src/gui/dialogs.rs`):
  - Sökvägs-inmatning
  - Validering (filen måste finnas)
  - Stödda format visas (.mod, .xm, .s3m)
- **Automatisk vy-växling** till Sequencer efter import
- **Status-toast** visar filnamn och antal samples

#### Beroenden

- `xmrs = { version = "0.9", features = ["std", "import"] }`

---

## [0.40.0] - 2025
### Added - The Hybrid Tracker

En modern tracker-arkitektur med View Adapter-mönster för framtida Piano Roll-stöd.

#### Datastruktur (`src/sequencer/pattern.rs`)

- **`TrackCell` enum** - Cell-baserad representation för tracker-style editing:
  - `Empty` - Tom cell
  - `Note { pitch, instrument, velocity }` - Not-event
  - `NoteOff` - Not-avstängning (`===`)
  - `Effect { command, value }` - Effekt-cell

- **`TrackerGrid`** - Grid-baserad lagring (rows × tracks):
  - `get()` / `set()` / `clear()` - Cell-operationer
  - `resize()` - Dynamisk storlek
  - `effects()` - Effekt-kolumner per cell

- **Pattern dual representation**:
  - `notes: Vec<Note>` - Piano roll-format (start, duration)
  - `grid: Option<TrackerGrid>` - Tracker-format (lazy-initialized)
  - `sync_grid_from_notes()` / `sync_notes_from_grid()` - Synkronisering

#### View Adapter (`src/sequencer/view/render.rs`)

- **`ColumnType`** enum: `RowIdx`, `Note`, `Instrument`, `Volume`, `EffectType`, `EffectValue`
- **`render_cell_text()`** - Returnerar `Cow<'static, str>` för noll-allokering på statiska strängar
- **`cell_color()`** - Färgbestämning baserat på cell-typ och cursor
- **`draw_track_cell()`** - Ny rendering via View Adapter
- **`draw_tracker_grid_from_pattern()`** - Optimerad rendering direkt från TrackerGrid

#### Input & Fokus (`src/sequencer/view/input.rs`)

- **`TrackerCommand`** enum - Engine-kommunikation (SetNote, SetNoteOff, ClearCell, etc.)
- **`TrackerCursor`** - Cursor-position (row, track, column)
- **`key_to_semitone()`** - Piano-keyboard layout (Z=C, S=C#, Q=C+12)
- **`hex_char_to_value()`** - Hex-inmatning för instrument/volym

#### Prestanda

- Virtual scrolling via `egui_extras::TableBuilder`
- `Cow<str>` för statiska strängar (---,  ===, ..) undviker heap-allokering
- Beat-markering var 4:e rad

#### Nya tester

- `test_track_cell_creation`
- `test_tracker_grid_basic`
- `test_tracker_grid_resize`
- `test_pattern_grid_sync`
- `test_pattern_set_cell`
- `test_grid_note_off_handling`
- Input-tester: `test_key_to_semitone`, `test_to_midi_note`, `test_hex_char_to_value`

---

## [0.39.0] - 2025
### Changed - Eliminera Duplicerade Typnamn

Alla strukturer och typer med samma namn i olika moduler har bytt namn till unika, beskrivande namn.

- **DSP-typer:**
  - `dsp::FilterType` → `SvfFilterType` (SVF = State Variable Filter)

- **Sequencer-typer:**
  - `sequencer::InstrumentId` → `SeqInstrumentId` (+ type alias för bakåtkompatibilitet)
  - `sequencer::InstrumentParam` → `AutoInstrumentParam` (automation-relaterad)

- **Engine shared state:**
  - `shared_state::MeterState` → `SharedMeterState` (trådsäker version)
  - `shared_state::TransportState` → `SharedTransportState` (trådsäker version)

- **Patch-serialisering:**
  - `patch::ModuleType` → `PatchModuleType`
  - `params::to_module_type()` → `to_patch_module_type()`

- **GUI widgets:**
  - `widgets::Port` → `PortWidget`
  - `widgets::PortType` → `WidgetPortType`
  - `widgets::PortDirection` → `WidgetPortDirection`
  - `patch_editor::VisualizerType` → `PaletteVisualizerType`

- **SampleRate:**
  - Behålls som två typer: `audio::SampleRate(u32)` för hårdvara, `types::SampleRate(f32)` för DSP
  - `From`-implementationer finns för konvertering mellan dem

### Why
Eliminerar förvirring vid import och gör det tydligt vilken typ som avses i varje kontext.

---

## [0.38.0] - 2025
### Added - Typsäkra Newtypes i Core Traits

Fullständig typning av `ProcessContext`, `PolyModule` och `AudioEffect` med newtype-mönstret.

- **Nya typer (`src/types/`):**
  - `BeatPosition` - Position i beats (musikalisk tid, f64)
    - Metoder: `bar()`, `beat_in_bar()`, `to_seconds()`, `quantize()`, `advance_samples()`
    - Konstant: `ZERO`
  - `Velocity` - MIDI velocity (0.0-1.0)
    - Metoder: `from_midi()`, `to_midi()`, `curve()`, `scale()`, `lerp()`
    - Konstanter: `ZERO`, `MAX`, `DEFAULT`, `PIANO`, `FORTE`
  - `MidiChannel` - MIDI-kanal (1-16)
    - Metoder: `as_u8()`, `as_index()`
    - Konstanter: `CH1`, `DRUMS`

- **Uppdaterad `ProcessContext`:**
  - `samples: usize` → `samples: SampleCount`
  - `position_beats: f64` → `position_beats: BeatPosition`

- **Uppdaterad `PolyModule` trait:**
  - `note_on(_note: MidiNote, _velocity: f32)` → `note_on(_note: MidiNote, _velocity: Velocity)`
  - `set_sample_rate(_sample_rate: f32)` → `set_sample_rate(_sample_rate: SampleRate)`

- **Uppdaterad `AudioEffect` trait:**
  - `set_mix(mix: f32)` → `set_mix(mix: NormalizedValue)`
  - `get_mix() -> f32` → `get_mix() -> NormalizedValue`
  - `tail_samples() -> usize` → `tail_samples() -> SampleCount`
  - `set_sample_rate(_sample_rate: f32)` → `set_sample_rate(_sample_rate: SampleRate)`

### Changed
- Alla 8 effekter uppdaterade med typsäkra signaturer
- Alla moduler som implementerar `PolyModule` uppdaterade
- `VoiceState`, `VoiceAllocator`, `Voice` använder nu `Velocity`
- Engine-filer använder typsäkra typer genomgående

---

## [0.37.0] - 2025
### Added - ValueRange Typ för Parameterhantering

- **Ny `ValueRange`-typ (`src/types/range.rs`):**
  - Kapslar in `min`, `max`, `default` i en enda typ
  - Fördefinierade konstanter: `UNIT`, `UNIT_ZERO`, `UNIT_ONE`, `BIPOLAR`, `PERCENT`, `TOGGLE`
  - Konstruktorer: `new()`, `symmetric()`, `from_min()`, `from_max()`
  - Metoder: `span()`, `contains()`, `clamp()`, `normalize()`, `denormalize()`, `lerp()`

- **Uppdaterad `ParameterDescriptor`:**
  - Ersatte separata `min`, `max`, `default` fält med `range: ValueRange`
  - Ny `value_range()` builder-metod
  - Bakåtkompatibla accessor-metoder: `min()`, `max()`, `default_value()`

- **Uppdaterad `ResponseCurve`:**
  - `normalize()` och `denormalize()` tar nu `ValueRange` istället för separata parametrar

### Changed
- GUI widgets (`Knob`, `module_panel`, `patch_editor`) använder nu `param.range.default/min/max`
- `ParameterWidget` i `src/ui/mod.rs` använder `range.normalize()` och `range.span()`

---

## [0.36.1] - 2025
### Changed - Kompaktare Modulstorlekar

- **Minskade modulstorlekar för att matcha kompakta widgets:**
  - Min bredd: 180px → 140px
  - Min höjd: 100px → 80px
  - Auto-layout gap: 10px → 8px
  - Modul X-offset: 210px → 160px
  - Modul Y-offset: 320px → 200px

- **Uppdaterade visualizer-storlekar:**
  - Meter: 80x100 → 60x80
  - Oscilloscope: 160x80 → 120x60
  - ADSR: 140x50 → 120x50

---

## [0.36.0] - 2025
### Added - ADSR Envelope Editor & Kompakta Knobs

- **Interaktiv EnvelopeEditor (`src/gui/widgets/envelope.rs`):**
  - Draggbara kontrollpunkter för Attack, Decay, Sustain, Release
  - Grid-bakgrund (5x5 linjer) för visuell referens
  - Tooltips visar värden vid hover/drag (ms/s för tid, % för sustain)
  - Total ljudtid (Σ A+D+R) visas i övre högra hörnet
  - Dynamisk skalning anpassar sig efter faktiska värden
  - Glow-effekt runt aktiva kontrollpunkter

- **Kompakta Knobs (`src/gui/widgets/knob.rs`):**
  - Storlek minskad: 72px → 36px (default), 56px → 28px (small), 88px → 48px (large)
  - Värde visas nu som tooltip istället för text i mitten
  - Borttagen yttre ram för kompaktare utseende
  - Arc-bredd och indikator skalas med knob-storlek

- **Återanvändbar Tooltip-modul (`src/gui/widgets/tooltip.rs`):**
  - `draw_value_tooltip()` - generell tooltip på valfri position
  - `draw_tooltip_right_of()` - för knobs (höger om cirkeln)
  - `draw_tooltip_above()` - för envelope-punkter
  - Ritas på `Order::Tooltip`-lagret för att alltid visas överst

- **Förbättrad Port-layout (`src/gui/patch_editor.rs`):**
  - Inputs vänsterställda, outputs högerställda med flexibelt mellanrum
  - Mindre labels (9px) för kompaktare vy
  - Tooltips med fullständig beskrivning vid hover

- **Förbättrade Topbar-knappar:**
  - Power: `●`/`○` med grön/grå färg och detaljerad hover-text
  - Connectivity: `◆`/`◇` med färgkodning och förklaringar
  - Större klickyta (20x20px) för bättre användbarhet

### Changed
- ADSR-moduler använder nu EnvelopeEditor istället för sliders
- Endast knob-parametrar (Vel Sens, kurvor) visas under envelope-editorn

---

## [0.35.0] - 2025
### Added - Tracker View (FastTracker II-inspirerad sequencer)
- **Ny TrackerViewState (`src/sequencer/view/state.rs`):**
  - Stark typ `RowIndex` för typsäker rad-navigering
  - `TrackerColumn` enum: Note, Instrument, Volume, EffectType, EffectValue
  - `TrackerViewState` med: cursor_row, cursor_track, cursor_column, octave, step_size, etc.
  - Hjälpmetoder: `cursor_down/up/left/right`, `ensure_cursor_visible`, `octave_up/down`

- **Tracker Grid Rendering (`src/sequencer/view/render.rs`):**
  - Använder `egui_extras::TableBuilder` för virtuell scrollning
  - `draw_tracker_grid()` - renderar pattern med färgkodade celler
  - `TrackerColors` - anpassningsbara färger för tracker-vyn
  - Stöd för Note, Instrument, Volume och Effect-kolumner

- **Sequencer View (`src/gui/views/sequencer.rs`):**
  - Toolbar med: Transport (⏮▶⏹⏺), Octave-väljare (F1/F2), Step-väljare, Follow-toggle
  - Full tangentbordsnavigering: Piltangenter, Home/End, PageUp/Down, Tab
  - Piano-tangentbordslayout för noter: Z=C, S=C#, X=D... Q=C+1, W=D+1, etc.
  - Placeholder för "No Song" med kortkommandon

- **Beroenden:**
  - Lagt till `egui_extras = "0.33"` för TableBuilder

---

## [0.34.11] - 2025
### Fixed - Auto Layout: ADSR/Modulation-moduler håller sig inom vyn
- **Korrigerad beräkning av modulation-radens Y-position:**
  - `mod_row_index` sätts nu korrekt till `main_rows` (efter alla huvudmoduler)
  - `mod_base_y` begränsas med `.min(max_y)` för att garantera att modulen håller sig inom canvas
- **ADSR/Envelope-moduler överlappar inte längre pianot/keyboardet**

---

## [0.34.10] - 2025
### Fixed - Auto Layout: Moduler håller sig garanterat inom vyn
- **Modulhöjden anpassas nu automatiskt:**
  - Om alla rader inte får plats med MIN_MODULE_HEIGHT (140px), krymps modulerna
  - Absolut minimum 60px höjd för att alltid passa
  - Formeln: `((available_height - GAP) / total_rows - GAP).max(60.0)`
- **Undersidan av moduler går aldrig utanför canvas:**
  - `max_y` beräknas som `available_rect.max.y - module_height - GAP`
  - `clamp_pos()` garanterar att modulens position + höjd alltid är inom vyn
- **Inga moduler överlappar keyboard/piano**

---

## [0.34.9] - 2025
### Improved - Auto Layout med strikta gränser
- **Okopplade moduler hanteras separat:**
  - Moduler utan kopplingar placeras nu i högra kolumnen
  - Staplas vertikalt för att undvika överlappning
- **Strikt gränskontroll:**
  - `clamp_pos()` funktion säkerställer att ALLA moduler håller sig inom canvas
  - Inga moduler kan överlappa pianot/keyboardet
  - `max_x` och `max_y` beräknas från modulstorlek
- **Förbättrad modulkategorisering:**
  - Kopplade huvudmoduler: vänster-till-höger efter signalflöde
  - Kopplade modulationsmoduler: under deras mål
  - Okopplade moduler: egen kolumn till höger
- **Nya tester:**
  - `test_disconnected_modules_in_corner` - verifierar att okopplade moduler placeras rätt
  - `test_linear_chain_within_bounds` - verifierar gränser

---

## [0.34.8] - 2025
### Improved - Auto Layout fyller hela vyn
- **Algoritmen omskriven för att dynamiskt beräkna modulstorlek:**
  - Modulbredden beräknas så att alla kolumner fyller tillgänglig bredd
  - Modulhöjden beräknas så att alla rader (inklusive modulation) fyller tillgänglig höjd
  - Minimumstorleks-begränsningar: 150x120 px
- **Moduler håller sig nu inom vyn:**
  - Canvas-rektangeln (exklusive sidopaneler och keyboard) används som begränsning
  - Modulation placeras under huvudsignalvägen med 25px gap
- **Förenklad API:**
  - `calculate_layout()` tar nu `Rect` direkt istället för `LayoutConfig`
  - Alla beräkningar sker internt baserat på tillgänglig yta

---

## [0.34.7] - 2025
### Improved - Auto Layout toolbar och höjdberäkning
- **Auto Layout-knappen ritas nu i foreground layer** - syns alltid överst, även när moduler dras över
- **Förbättrad layoutalgoritm:**
  - Moduler på samma djup-nivå staplas nu vertikalt (samma kolumn)
  - Djup-nivåer fortsätter till nästa kolumn
  - Om kolumnerna tar slut, fortsätter layouten på en ny "rad av kolumner"
- **Korrekt höjdhantering:**
  - Modulationsmoduler placeras baserat på faktiskt antal rader som används
  - Begränsning till `max_main_rows + 1` för att undvika att modulation hamnar för långt ner

---

## [0.34.6] - 2025
### Improved - Auto Layout respekterar tillgängligt utrymme
- **Algoritmen tar nu hänsyn till:**
  - Den tillgängliga canvas-ytan (exklusive sidopaneler)
  - Modulernas storlek (200x180 px)
  - Gap mellan moduler (20 px)
  - Radbrytning när kolumner inte får plats
- **Nya konfigurations-parametrar:**
  - `area_min`, `area_max` - Tillgänglig layoutyta
  - `module_size` - Modulstorlek för beräkningar
  - `gap_x`, `gap_y` - Avstånd mellan moduler
  - `modulation_gap` - Extra avstånd till modulations-raden
- **Moduler överlappar inte längre** - algoritmen placerar moduler i ett rutnät
- **Canvas-rect skickas nu till layoutfunktionen** för korrekt placering

---

## [0.34.5] - 2025
### Fixed - Auto Layout-knappen fungerar nu
- **Problemet:** Auto Layout-knappen uppdaterade interna positioner men egui:s `Window`-widget ignorerade detta eftersom `default_pos()` bara sätter positionen första gången fönstret ritas.
- **Lösningen:** Lade till `needs_reposition: HashSet<ModuleId>` i `PatchEditor` som markerar moduler som behöver omplaceras. När en modul är markerad används `current_pos()` istället för `default_pos()`, vilket tvingar fönstret till den nya positionen.
- Auto Layout fungerar nu som förväntat - moduler flyttas till sina beräknade positioner baserat på signalflödet.

---

## [0.34.4] - 2025
### Added - Workspace GUI Navigation
- **gui/app/state.rs** - Nya navigations-enums:
  - `AppView` - Rack/Sequencer/Mixer vy-val med `icon()` och `label()` metoder
  - `TopPanel` - None/Midi/Engine för expanderbara paneler (förberett för framtida funktionalitet)
- **gui/views/layout.rs** - Top bar och drawer komponenter:
  - `TopBarContext` struct för att skicka state till top bar
  - `draw_top_bar()` - Komplett menybar med vy-flikar
  - `draw_top_drawer()` - Expanderbar panel för MIDI/Engine (förberedd)
- **gui/views/rack.rs** - Rack-vy komponent:
  - `RackContext` struct för instrument rack state
  - `draw_instrument_rack()` och `draw_empty_state()` funktioner
- **gui/views/sequencer.rs** - Sequencer-vy (placeholder)
- **gui/views/mixer.rs** - Mixer-vy (placeholder)

### Changed - View Routing
- **egui_backend.rs** - Vy-router implementerad:
  - `active_view: AppView` fält i `SynthApp`
  - Vy-flikar i menyraden (🎛️ Rack, 🎹 Sequencer, 🎚️ Mixer)
  - Toolbar och instrument rack visas endast i Rack-vyn
  - Match-statement för vy-routing i CentralPanel

---

## [0.34.3] - 2025
### Improved - Prestandaoptimering & Slutfört Type Hardening

**Del 1: InternPool optimering (prestandakritisk)**
- `PortName` har nu **compile-time konstanter** (`PortName::IN`, `PortName::OUT`, etc.)
- **Ingen låsning krävs** för standardportnamn i ljudtråden
- 23 fördefinierade port-IDn: `IN`, `OUT`, `IN_L`, `IN_R`, `OUT_L`, `OUT_R`, `FREQ`, `FREQ_CV`, `GATE`, `CUTOFF_CV`, `RESONANCE_CV`, `PWM`, `FM`, `PM`, `SYNC`, `LEVEL`, `PAN`, `RATE_CV`, `CV`, `PAN_CV`, `LEFT`, `RIGHT`, `VELOCITY`
- Gamla metoder (`input()`, `output()`, etc.) är deprecated

**Del 2: InputPorts::get optimering**
- `InputPorts::get(name: PortName)` - direkt `u32`-jämförelse, **O(1) utan strängjämförelse**
- `InputPorts::get_str(name: &str)` - convenience-metod för dynamiska portnamn
- Alla moduler uppdaterade att använda `PortName::*` konstanter

**Del 3: Nya state enums**
- `FreezeState` - `Unfrozen`/`Frozen` för reverb/oscilloscope freeze
- `Polarity` - `Normal`/`Inverted` med `multiplier()` metod
- `TempoSyncState` - alias för `SyncMode`

**Del 4: Modulmigrering bool → enum**
- `KeyboardPanner`: `invert: bool` → `polarity: Polarity`
- `Delay`: `tempo_sync: bool` → `tempo_sync: TempoSyncState`
- `Mixer`: `mute: bool` → `mute_state: MuteState`, `limit: bool` → `limit_mode: LimitMode`

**Del 5: Parametrar uppdaterade**
- `KeyboardPannerParam::Invert(Polarity)` istället för `bool`
- Alla `as_f32()` och `with_f32()` metoder uppdaterade

**Del 6: SamplePlayer (redan optimerad)**
- `Arc::clone` sker redan innan for-loopen - korrekt implementerad

---

## [0.34.2] - 2025
### Improved - Fas 3 & 4: Type Hardening Complete
- **Fas 3 - Arkitektur & Prestanda:** Verifierad - redan optimerad
  - `PortName` (internad sträng) redan implementerad i `types/interned.rs`
  - `EffectCommand` enum redan typat i `sequencer/effects.rs`
  - HashMap-lookups använder `&str` utan allokeringar i audio-tråden
- **Fas 4 - FilterState wrapper:**
  - `Filter` (SVF) använder nu `FilterState` istället för råa `f32`
  - `ic1eq` och `ic2eq` integrator-state är nu typsäkra
  - Konsistent med `LadderFilter` som redan använde `FilterState`

---

## [0.34.1] - 2025
### Improved - Fas 2: Sampling & Uppspelning
- **types/sample.rs** - `Interpolation` enum utökad med GUI-stöd:
  - `ALL` konstant med alla 7 interpolationslägen
  - `name()`, `id()`, `index()`, `from_index()` metoder
  - `to_choices()` för GUI-dropdown
- **types/state.rs** - `NoteReleaseState` enum tillagd:
  - `Held` - Ton hålls, normal uppspelning med looping
  - `Released` - Ton släppt, spelar till slut utan looping
  - Metoder: `is_released()`, `is_held()`, `release()`, `hold()`
- **SamplePlayer** - Refaktorerad med semantisk state:
  - `note_release_state: NoteReleaseState` istället för `releasing: bool`
  - Tydligare kodintention vid note-on/note-off hantering

---

## [0.34.0] - 2025
### Added - Type Hardening: Semantic State Enums
- **types/state.rs** - Ny modul för semantiska tillstånds-enums som eliminerar "Boolean Blindness"
  - `EnableState` - Enabled/Disabled
  - `MuteState` - Unmuted/Muted
  - `SoloState` - Normal/Solo
  - `BypassState` - Active/Bypassed
  - `SyncMode` - Free/TempoSync
  - `RetriggerMode` - Continue/Retrigger
  - `ClipMode` - Off/Soft/Hard
  - `LimitMode` - Enabled/Disabled
- **types/sample.rs** - Flyttade `LoopMode` och `ReleaseMode` från engine/params/modules.rs
  - Tillhör nu types-modulen där sampling-relaterade typer samlas
  - Bakåtkompatibla re-exports i engine/params/modules.rs

### Changed - Module Refactoring with State Enums
- **SharedEngineState** - `ModuleStateSnapshot` använder nu:
  - `bypass_state: BypassState` istället för `bypassed: bool`
  - `mute_state: MuteState` istället för `muted: bool`
  - `solo_state: SoloState` istället för `solo: bool`
- **Amplifier** - `clip_mode: ClipMode` istället för `soft_clip: bool`
  - Stöd för Off, Soft (tanh), och Hard clipping
- **LFO** - Använder nu:
  - `sync_mode: SyncMode` istället för `tempo_sync: bool`
  - `retrigger_mode: RetriggerMode` istället för `retrigger: bool`
- **StereoOutput** - Använder nu:
  - `mute_state: MuteState` istället för `muted: bool`
  - `limit_mode: LimitMode` istället för `limit_enabled: bool`

---

## [0.33.27] - 2025
### Improved - GUI Views Module
- **gui/views/** - Ny modul för återanvändbara GUI-komponenter
  - `views/master_effects.rs` - `MasterEffectParams` och `MasterEffectUiState` typer
  - `views/meters.rs` - `draw_meter()` och `draw_meter_horizontal()` funktioner
  - Reducerade egui_backend.rs från 2698 till 2482 rader (~216 rader flyttade)

---

## [0.33.26] - 2025
### Improved - Code Organization Refactoring
- **gui/input.rs** - Ny modul för keyboard input hantering
  - Extraherade `KEY_MAP` konstant och `handle_keyboard_input()` från egui_backend.rs
  - `KeyboardInputState` struct för att samla input state innan mutation
  - Reducerade egui_backend.rs med ~65 rader
- **SynthEngine command handlers** - Refaktorerade 620-raders `handle_command()` match-block
  - Extraherade 30+ handler-metoder grupperade i kategorier:
    - Instrument management (add, remove, set params, channel, enabled, solo)
    - Note control (note on, note off, all notes off)
    - MIDI controllers (pitch bend, mod wheel, aftertouch, poly aftertouch)
    - Global parameters (master volume, glide time)
    - Voice/module parameters (set voice param, set module param)
    - Reset/clear (reset, clear all modules)
    - Effects (bypass, effect param, enabled, visualizers, add/remove effect)
    - Modular routing (add/remove module, connect, disconnect)
  - Tydligare kodseparation och enklare underhåll

---

## [0.33.25] - 2025
### Added - StereoSample Type & DSP Module
- **StereoSample** - Ny typ i `types/audio.rs` för stereo-samples
  - Ersätter `(f32, f32)` och `[f32; 2]` för stereosignaler
  - Metoder: `new()`, `from_mono()`, `apply_gain()`, `apply_pan()`, `to_mono()`, `mix()`, `soft_clip()`, `hard_clip()`, `peak()`
  - Implementerar `Add`, `Sub`, `Mul<f32>`, `From<(f32, f32)>`, `From<[f32; 2]>`
- **src/dsp/** - Ny modul för återanvändbara DSP-primitiver
  - `dsp/oscillators.rs` - `poly_blep()` och `poly_blep_integrated()` för band-limiterade vågformer
  - `dsp/filters.rs` - `SvfCoeffs` och `BiquadCoeffs` för filterberäkningar
  - `dsp/delay.rs` - `DelayLine` och `InterpolatedDelayLine` för delay-effekter

### Improved - Module Refactoring
- **KeyboardPanner** - Använder nu `StereoSample::apply_pan()` internt
- **StereoOutput** - Refaktorerad att använda `StereoSample` för beräkningar
- **Delay effect** - Använder nu `StereoSample` för stereobearbetning
- **Reverb effect** - Använder nu `StereoSample` för stereobearbetning
- **Oscillator** - Använder nu `dsp::oscillators::poly_blep()` istället för lokal metod

---

## [0.33.24] - 2025
### Improved - Type Methods & Sequencer Types
- **MidiNote::transpose** - Flyttade transponeringslogik till typen, returnerar `Option<MidiNote>`
- **Pitch::transpose** - Sequencer-typ uppdaterad till `Semitones`, returnerar `Option<Pitch>`
- **PatternPlacement** - `transpose` nu `Semitones`, `gain` nu `Gain`
- **TempoChange/Song** - `bpm` och `default_tempo` nu `Bpm` istället för `f32`
- **Pattern::generate_events** - Använder nu `Semitones` för transponering
- **Instrument::transpose_note** - Delegerar nu till `MidiNote::transpose`

---

## [0.33.23] - 2025
### Improved - Strict Type Hardening
- **StereoBalance** - Ny typ för stereopanorering med constant-power gains
- **KeyboardPanner** - Använder nu `MidiNote`, `BipolarValue`, `StereoBalance` istället för primitiver
- **BodyResonance** - Filterstate arrays använder nu `FilterState` istället för `f32`
- **Instrument transpose** - Använder nu `Semitones` istället för `i8`
- **InstrumentParam::Transpose** - Uppdaterad till `Semitones`
- **Oscilloscope** - `mix` och `sample_rate` använder nu `NormalizedValue` och `SampleRate`
- **LevelMeter** - `mix` och `sample_rate` använder nu `NormalizedValue` och `SampleRate`
- **GUI InstrumentUiState** - `transpose` använder nu `Semitones`

---

## [0.33.22] - 2025
### Optimized - GUI Rendering with egui Shape Primitives
- **Kablar** - Ersatte manuell Bézier-loop (32 segment × 3 lager) med `CubicBezierShape`
- **Oscilloskop** - Ersatte ~200 `line_segment()` med en `Shape::line()`
- **Waveform-väljare** - Ersatte 31 `line_segment()` per ikon med `Shape::line()`
- **ADSR Envelope** - Ersatte 4 `line_segment()` med `Shape::line()`
- **Draw Call Batching** - GPU kan nu rita alla punkter i en operation
- **Automatisk LOD** - `CubicBezierShape` hanterar detaljnivå automatiskt

---

## [0.33.21] - 2025
### Improved - Enhanced Sample Player
- **PlaybackState** - Ersatte `bool` med typat enum för tydligare state
- **SampleName** - Newtype för sample-namn istället för rå `String`
- **ReleaseMode** - Tre lägen: `Immediate`, `PlayToEnd`, `PlayToLoop` för flexibel note-off hantering
- **Velocity Sensitivity** - Parameter för hur mycket velocity påverkar volym (0-100%)
- **Nya interpolationer** - Hermite, Lagrange, Sinc8, Sinc16 för högkvalitativ uppspelning
- **Loop Crossfade** - 0-50ms crossfade vid loop-punkter för klickfri looping
- **Root Key Detection** - Automatisk detektion av grundton från filnamn (t.ex. "Piano_C3.wav")
- **WaveformOverview** - Pre-beräknad waveform för effektiv visualisering
- **PlaybackPositionBuffer** - Atomic position buffer för lock-free GUI-synk

### Changed
- Konsoliderade `LoopMode` - endast en definition i params/modules.rs
- Tog bort `loop_mode` från `Sample` struct (hör till spelaren, inte samplen)

---

## [0.33.20] - 2025
### Added - Sample Player & Sample Manager
- **SamplePlayer** - Ny modul för uppspelning av WAV-samples
  - Pitch tracking (transponerar automatiskt baserat på spelade noter)
  - Loop modes: Off, Forward, Backward, PingPong
  - Start/End positions för sample trimming
  - Loop Start/End för preciserad loop-region
  - Speed-kontroll (0.1x - 4.0x)
  - Interpolation: Nearest, Linear, Cubic (Catmull-Rom)
  - Stereo och mono-samples stöds
- **SampleManager** - GUI-thread sample loader med caching
  - Laddar WAV-filer (8/16/24/32-bit int, 32-bit float)
  - Cache förhindrar dubbel-laddning av samma fil
  - Thread-safe via `Arc<Sample>`
- **Nya typer** - `SampleValue`, `PlaybackPosition`, `SampleIndex`, `PlaybackSpeed`, `ChannelMode`, `Interpolation`, `PlaybackDirection`
- **hound** - Nytt beroende för WAV-läsning

---

## [0.33.19] - 2025
### Added - Improved Cables & Auto-Layout
- **Kablar med gravitation** - Kablar hänger nedåt med naturlig "sag" (15% av avståndet)
- **Skuggor** - Svart skugga under kablar för djupkänsla
- **Semi-transparens** - Kablar är delvis genomskinliga (alpha 180)
- **Highlight på hover** - Röd glow-effekt från theme().colors.accent_red
- **Dragging-kablar** - Mindre sag (5%) för responsiv känsla
- **Auto-Layout** - Ny knapp "📐 Auto Layout" som organiserar moduler
  - Vänster→höger baserat på signalflöde (BFS från sources)
  - Envelopes/LFOs placeras under huvudsignalvägen
  - Konfigurerbara avstånd via LayoutConfig

---

## [0.33.18] - 2025
### Fixed - Parameter Routing for Arbitrary Modules
- **SetModuleParameter** används nu för alla voice-moduler istället för SetVoiceParameter
- Fixar parameter-routing för moduler utanför PolyModule enum (env-3, amp-2, sub-1, nse-1, kbp-1, etc)
- **Grand Piano patch** fungerar nu korrekt med alla 3 envelopes
- Tog bort redundant get_voice_module_for_param funktion
- Rensade oanvända imports (PolyModule, Param)

---

## [0.33.17] - 2025
### Changed - Physical Modeling Cleanup
- **Removed** StringResonator, ResonatorBank, VelocityMapper (fungerade inte korrekt)
- **KeyboardPanner** - Not-baserad stereopanorering nu registrerad i GUI-menyn
- **BodyResonance** - Resonanskropp-simulering nu registrerad i GUI-menyn
- **MechanicalNoise** - Mekaniska ljud nu registrerad i GUI-menyn
- **Grand Piano patch** - Uppdaterad med KeyboardPanner för stereo-imaging
- **Physical-menyn** i modulpaletten med de 3 fungerade modulerna

---

## [0.33.16] - 2025
### Added - Physical Modeling Modules
- **StringResonator** - Karplus-Strong string synthesis med inharmonicitet och dämpning
- **ResonatorBank** - Sympatisk resonans med 1-12 avstämbara strängar
- **KeyboardPanner** - Not-baserad stereopanorering för piano-liknande stereo
- **BodyResonance** - Resonanskropp-simulering (soundboard)
- **VelocityMapper** - Velocity-kurvor (Linear, Soft, Hard, S-Curve, Fixed)
- **MechanicalNoise** - Mekaniska ljud (tangent ner/upp, pedal, hammare)
- **ModuleCategory::PhysicalModeling** - Ny kategori för modulpalett

---

## [0.33.15] - 2025
### Added - Keyboard Splitting & MIDI Learn
- **KeyRange** - Ny typ för att definiera vilka noter ett instrument svarar på (keyboard splitting)
- **LearnState** - State machine för MIDI learn (Idle, WaitingForLowNote, WaitingForHighNote)
- **Transpose** - Semitone-offset per instrument (-24 till +24)
- **Instrument Rack UI** - Ny rad med Range-visning, Learn-knapp, Full-knapp, Transpose-kontroll
- **KeyRangeLearned event** - Engine skickar event till GUI när range lärs in
- **note_on/note_off** - Kollar key_range och applicerar transpose

---

## [0.33.14] - 2025
### Improved - GUI Styling & Theme Consistency
- **Master FX Sliders** - Bättre synlighet med mörkare bakgrund och tydlig kontrast
- **WidgetStyle** - Nya fält: knob_arc_segments, slider_rail_height, slider_handle_radius
- **knob.rs** - Använder nu theme().style istället för hårdkodade värden
- **meter.rs** - Använder nu theme().style istället för hårdkodade värden

---

## [0.33.13] - 2025
### Added - Attenuverters for CV Inputs
- **Filter CutoffMod** - Ny "CV Amt" parameter (-1.0 till +1.0) för cutoff CV
- **Oscillator FmAmount** - Ny "FM Amt" parameter (-1.0 till +1.0) för FM input
- Tog bort MIDI debug-spam

---

## [0.33.12] - 2025
### Added - Theme System
- 8 färgteman: Dark, Light, Vintage, Neon, Studio, Dracula, Monokai, Solarized Dark
- Tema-väljare i Settings, WidgetStyle för konsistent styling

---

## [0.33.11] - 2025
### Changed - InputPorts Refactor
- `PolyModule::process()` använder `InputPorts` wrapper istället för HashMap
- Eliminerar HashMap-allokering per audio frame

---

## [0.33.10] - 2025
### Fixed - Realtime Audio Allocations
- Eliminerade `AudioBuffer::new()` i audio thread (~187 allok/sek)
- `Connection` använder `PortName` (Copy) istället för String

---

## [0.33.9] - 2025
### Added - Bypass-knappar
- Power-knapp (⏻) i varje moduls header, bypassade moduler dimmas till 40%

---

## [0.33.8] - 2025
### Added - Master FX Parameters
- Resizable sidopanel, fullständiga parameterkontroller för alla 8 effekttyper

---

## [0.33.7] - 2025
### Added - Master FX Sidebar
- Kollapsbar effektlista i sidopanelen med bypass/remove per effekt

---

## [0.33.6] - 2025
### Added - Mixer & Master Bus
- Solo-knapp, global master effects chain, soft clipper per instrument

---

## [0.33.5] - 2025
### Fixed - Eliminated unwrap/expect
- Fixade 112 Clippy-varningar för unwrap/expect i produktionskod

---

## [0.33.4] - 2025
### Maintenance
- Rensade 29 onödiga clippy allows, uppdaterade 15 beroenden

---

## [0.33.3] - 2025
### Refactored - Clippy Pedantic
- Konfigurerade ~70 pedantic/nursery lints för synth-lämpliga undantag

---

## [0.33.2] - 2025
### Refactored - Best Practices
- `#[must_use]` på transformationsmetoder, tog bort global `#![allow(dead_code)]`
- Konverterade error-typer till thiserror, reducerade unsafe från 5 till 1 block

---

## [0.33.1] - 2025
### Refactored - Idiomatic Iterators
- Konverterade for-loopar till iteratorer utanför hot path, behöll for-loopar i DSP

---

## [0.33.0] - 2025
### Added - Type System Extensions
- `PortName` interning för zero-allocation, `FilterState` DSP-metoder
- `Hertz`, `NormalizedValue`, `MidiChannel`, `BeatDivision` extensions

---

## [0.32.25] - 2025
### Fixed - Instrument Channel Isolation
- Standardinstrument använder CH1 istället för OMNI
- Real-time safe sequencer (pre-allokerad event buffer)

---

## [0.32.24] - 2025
### Refactored - DSP Type Hardening
- `FilterState` i Reverb/Phaser, `Hertz::to_tan_coeff()`, `Milliseconds::to_samples()`

---

## [0.32.23] - 2025
### Fixed - PatchEditor GUI ID Collision
- Window ID inkluderar instrument_id för unika GUI-identifierare
- LadderFilter och NoiseGenerator använder `FilterState`

---

## [0.32.22] - 2025
### Refactored - Total Type Hardening
- `MidiNote` för PolyModule/EngineCommand, `SamplePosition`/`SampleCount` i Voice
- `BufferIndex` i Flanger/Chorus, `NormalizedValue` i SequencerTrack

---

## [0.32.21] - 2025
### Refactored - Type Safety and Enums
- `ModuleType::is_voice_module()`, unified `EffectChain` med `ChainSlot`
- Data-bärande `VoiceState` enum (Idle/Active/Releasing/Stealing)

---

## [0.32.20] - 2025
### Refactored - Per-Instrument Effects
- Flyttade effekter från global MasterBus till per-instrument EffectChain

---

## [0.32.19] - 2025
### Refactored - Per-Instrument PatchEditor
- Varje instrument äger sin egen PatchEditor, patch-laddning per instrument

---

## [0.32.18] - 2025
### Refactored - Per-Instrument Voice Architecture
- `voice_graph` ägs av Instrument istället för SynthEngine

---

## [0.32.17] - 2025
### Refactored - Architectural Terminology
- Renamed: RackView→PatchEditor, VoiceModule→PolyModule, EffectModule→AudioEffect
- SynthPart→Instrument, EffectChain→MasterBus

---

## [0.32.16] - 2025
### Added - Part Manager UI
- Multi-instrument support med Part Manager panel, MIDI-kanal per part

---

## [0.32.15] - 2025
### Improved - Knob Widget
- Värde visas i knob-cirkeln, centraliserad formatering i ParameterUnit
- Custom "Share Tech Mono" font

---

## [0.32.14] - 2025
### Added - MIDI Input Support
- Hardware MIDI via midir med GUI port-väljare, velocity visualization
- Type-safe MIDI parsing, pitch bend, mod wheel, aftertouch

---

## [0.32.13] - 2025
### Fixed - Stereo Output Parameters
- ModuleCategory::Output tillagd i parameter change handling

---

## [0.32.12] - 2025
### Removed - Performance Panel
- Tog bort GUI-komponenten, behöll engine-kommandona för framtida MIDI

---

## [0.32.11] - 2025
### Fixed - Real-time Parameter Updates
- `SetModuleParameter` uppdaterar nu voice_template + alla aktiva voices
- Tog bort 29 ogiltiga oscilloskop-kopplingar från patches

---

## [0.32.10] - 2025
### Fixed - Critical Audio Routing
- StereoOutput klassificeras som voice module, partiella inputs fungerar
- `ClearAllModules` rensar voice_template

---

## [0.32.9] - 2025
### Added - Dynamic Module Routing
- Moduler routas automatiskt till rätt graf baserat på typ

---

## [0.32.8] - 2025
### Refactored - Unified Voice/Graph
- Voice äger ModuleGraph istället för hårdkodad modullista (~400 rader borttaget)

---

## [0.32.7] - 2025
### Refactored - Unified Param Architecture
- `Param` enum med inbakade typade värden, tog bort `TypedValue`

---

## [0.32.6] - 2025
### Fixed - Dropdown Parameter Sync
- Dropdown-handlers skickar rätt TypedValue-variant

---

## [0.32.5] - 2025
### Added - Domain Types for Effects
- `Ratio` (kompression), `BeatDivision` (tempo-sync), `VoiceCount`

---

## [0.32.4] - 2025
### Fixed - Waveform Selection
- GUI skickar TypedValue::Waveform, tog bort noise från Oscillator (använd NoiseGenerator)

---

## [0.32.3] - 2025
### Fixed - CV Modulation Drift
- LadderFilter, LFO, Oscillator använder effective values utan att modifiera parametrar

---

## [0.32.2] - 2025
### Fixed - GUI/Engine Sync
- Startup använder patch_bridge::load_patch() för synkronisering

---

## [0.32.1] - 2025
### Fixed - Ghost Sound Bug
- ClearAllModules inaktiverar parts

---

## [0.32.0] - 2025
### Added - Type Safety & GUI
- Type-safe public APIs med Hertz, Cents, Gain, NormalizedValue
- "New Patch" i File-menyn

---

## [0.31.0] - 2025
### Added - GUI Module Support
- SubOscillator och NoiseGenerator i module palette
- 3 nya example patches

---

## [0.30.0] - 2025
### Added - DSP Improvements
- Envelope curves (attack_curve, decay_curve, release_curve)
- SubOscillator modul (sine, square, -1/-2 oktav)
- NoiseGenerator modul (white, pink, brown, blue, violet)

---

## [0.29.0] - 2025
### Refactored - Modular Patch Structure
- 16 patches extraherade till individuella filer i src/patches/

---

## [0.28.1] - 2025
### Added - Performance Fixes
- Velocity mapping till engine, tempo sync för Delay och LFO
- Module connectivity visualization (Connected/Orphaned/Disconnected)

---

## [0.28.0] - 2025
### Added - Performance Panel
- Pitch bend (spring-back), mod wheel, velocity mapping knobs

---

## [0.27.0] - 2025
### Added - Enhanced Oscilloscope & LFO Tempo Sync
- Oscilloskop med waveform history, time division, trigger modes
- LFO tempo sync med beat divisions

---

## [0.26.0] - 2025
### Added - Engine Events & CPU Tracking
- EngineEvent system med prioriterad kanal
- CPU usage tracking per modul

---

## [0.25.0] - 2025
### Added - Effect Bypass & Master Volume
- Effect bypass per slot, master volume control

---

## [0.24.0] - 2025
### Refactored - Hub Architecture
- EventHub för GUI-engine kommunikation, ersatte direkt polling

---

## [0.23.0] - 2025
### Added - Visual States & Animations
- ModuleVisualState för visuell feedback, cable animations

---

## [0.22.0] - 2025
### Added - Sequencer Engine
- SequencerEngine med transport, looping, note events

---

## [0.21.0] - 2025
### Added - Sequencer Data Model
- Song, Pattern, Note, Track, Automation strukturer

---

## [0.20.0] - 2025
### Added - Effect Chain & Visualizers
- MasterBus med insert effects och visualizers

---

## [0.19.0] - 2025
### Added - Oscilloscope Widget
- Real-time waveform display i GUI

---

## [0.18.0] - 2025
### Added - Level Meter Widget
- VU-meter med peak hold och gradient

---

## [0.17.0] - 2025
### Added - Patch Save/Load
- JSON-baserat patch-format med ModuleBuilder API

---

## [0.16.0] - 2025
### Added - Voice Allocator
- Polyfoni med voice stealing, mono/poly modes

---

## [0.13.2] - 2025
### Fixed - Command Queue Overflow
- Ökade COMMAND_BUFFER_SIZE, DroppedModule wrapper

---

## [0.13.1] - 2025
### Added - Pink Noise & Linear FM
- Pink noise (Voss-McCartney), Linear FM mode, velocity sensitivity

---

## [0.13.0] - 2025
### Added - Math Oscillator
- 18 algoritmer: SineFM, TanChaos, SuperSaw, WaveFolder, Lorenz, KarplusStrong, etc.
- 6 nya example patches

---

## [0.12.0] - 2025
### Initial Release
- Moduler: Oscillator, Filter, Envelope, LFO, Amplifier, Mixer
- Effekter: Delay, Reverb, Distortion, Chorus, Phaser, Flanger, Compressor, EQ
- 10 example patches, piano keyboard, visual cable connections
