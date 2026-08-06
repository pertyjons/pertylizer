# Unified egui theme architecture

> **Status:** planned.
>
> **Planning baseline:** Pertylizer `1d4139f1`, 2026-08-05.

## Goal

Make theming simpler, deterministic, and easier to extend by giving each kind
of visual data one clear owner:

- egui owns generic widget visuals, text styles, spacing, and interaction
  styling through `egui::Style` and `egui::Visuals`;
- Pertylizer owns named presets and synth-specific visual semantics such as
  signal, cable, meter, graph, and transport colours;
- `SynthApp` owns the selected and resolved theme state on the UI thread;
- application settings persist only a stable theme identity, not an egui
  implementation detail.

The result must preserve all current built-in presets while removing the
parallel mutable sources of truth represented by the global `Theme` lock and
the style installed in `egui::Context`.

## Non-goals

- Do not add a user-facing theme editor in this work.
- Do not serialize `egui::Style` or depend on its serialized representation.
- Do not make every layout dimension theme-dependent.
- Do not change audio behaviour or introduce theme access on the audio thread.
- Do not preserve the current internal theme API; the project does not require
  backward compatibility during active development.

## Current code and gap

`gui/theme.rs` currently combines five concerns in one global value:

- eight named and persisted presets;
- generic surface, text, widget-state, and selection colours;
- synth-specific signal, cable, and meter colours;
- font sizes and generic spacing;
- custom-widget dimensions and painting parameters.

Every preset uses the same `Fonts`, `Sizes`, `Spacing`, and `WidgetStyle`; only
its `Colors` differ. Several declared size and style fields are unused.

`setup_custom_style` already translates part of `Colors` into
`egui::Visuals`, and copies `slider_rail_height` into `egui::Spacing`. The
original values remain globally accessible through `theme()`, so generic
widgets frequently bypass the style egui already carries. This creates two
representations that must be kept synchronized and allows hard-coded values to
drift from the nominal theme values. For example, widget corner radii are
stored in `WidgetStyle` but are also configured with literal values when the
egui style is installed.

The global `RwLock<Theme>` is unnecessary for UI-owned state. It also hides
dependencies: custom painters call `theme()` internally instead of declaring
which synth-specific visuals and metrics they need.

egui's `Theme` type is not a replacement for Pertylizer presets. It selects
only a dark or light style slot. `Style` and `Visuals` are the correct owners
for generic styling, while Pertylizer still needs its named presets and domain
visuals.

## Target data flow

```text
AppSettings.theme_id
        |
        v
ThemeRegistry ----> ThemeDefinition
                          |
                          v resolve()
                   ResolvedTheme
                    |          |
                    |          +--> SynthVisuals --> custom painters
                    |
                    +--> egui::Style --> egui::Context --> normal widgets
```

`ThemeDefinition` is the single authoritative input. `egui::Style` and
`SynthVisuals` are derived outputs and are never edited independently.

## Theme identity and registry

Replace the serialized preset enum with a stable domain newtype and look up
definitions in a registry:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[must_use]
pub(crate) struct ThemeId(String);

pub(crate) struct ThemeDefinition {
    pub id: ThemeId,
    pub display_name: &'static str,
    pub scheme: egui::Theme,
    pub palette: ThemePalette,
}

pub(crate) struct ThemeRegistry {
    definitions: HashMap<ThemeId, ThemeDefinition>,
}
```

Built-in identifiers should be stable and namespaced, for example
`pertylizer.dark`, `pertylizer.light`, and `pertylizer.neon`. An unknown ID in
settings falls back to `pertylizer.dark` and produces the same kind of
non-fatal settings warning used for other recoverable configuration problems.

The registry is useful even before external theme files exist: the settings
schema no longer needs a new enum variant for every theme, IDs can remain
stable when display names change, and future theme providers have one explicit
registration boundary. Do not add dynamic loading as part of this migration.

## Semantic palette

Name colours by meaning rather than by hue. A call site should request a
danger, success, audio-signal, or selected colour; it should not request red,
green, cyan, or orange and thereby assume how every preset represents that
meaning.

```rust
pub(crate) struct ThemePalette {
    pub surfaces: SurfacePalette,
    pub text: TextPalette,
    pub actions: ActionPalette,
    pub status: StatusPalette,
    pub signals: SignalPalette,
    pub meters: MeterPalette,
}

pub(crate) struct SurfacePalette {
    pub canvas: Color32,
    pub panel: Color32,
    pub raised: Color32,
    pub control: Color32,
    pub border: Color32,
}

pub(crate) struct TextPalette {
    pub primary: Color32,
    pub secondary: Color32,
    pub muted: Color32,
}

pub(crate) struct ActionPalette {
    pub primary: Color32,
    pub selected: Color32,
}

pub(crate) struct StatusPalette {
    pub info: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
}

pub(crate) struct SignalPalette {
    pub audio: Color32,
    pub control: Color32,
    pub gate: Color32,
    pub midi: Color32,
}
```

Meter and graph palettes should contain only roles that cannot be expressed by
the status, action, or signal palettes. Where a cable and its port intentionally
represent the same signal type, derive both from `SignalPalette` instead of
maintaining two nearly identical colour sets.

The initial migration should preserve the current rendered colours. Renaming a
token must not silently change the visual design.

## Generic style belongs to egui

Resolve a definition from a fresh egui style for its colour scheme. Do not
clone the currently active global style because it may contain values from the
previous preset or the operating system's original theme.

```rust
impl ThemeDefinition {
    pub(crate) fn resolve(&self) -> ResolvedTheme {
        let mut style = self.scheme.default_style();

        apply_text_styles(&mut style);
        apply_spacing(&mut style.spacing);
        apply_visuals(&mut style.visuals, &self.palette);

        ResolvedTheme {
            scheme: self.scheme,
            style,
            synth_visuals: SynthVisuals::from_palette(&self.palette),
        }
    }
}
```

The mapping should have one implementation and cover at least:

| Theme role | egui owner |
|---|---|
| Primary text | `Visuals::override_text_color` or widget foregrounds |
| Muted text | `Visuals::weak_text_color` |
| Panel surface | `Visuals::panel_fill` |
| Raised/window surface | `Visuals::window_fill` |
| Control/text-edit surface | `Visuals::extreme_bg_color` and `text_edit_bg_color` |
| Faint/striped surface | `Visuals::faint_bg_color` |
| Warning and danger | `Visuals::warn_fg_color` and `error_fg_color` |
| Widget states | `Visuals::widgets` |
| Selection | `Visuals::selection` |
| Window and widget borders | egui strokes |
| Corner radii | egui window, menu, and widget corner radii |
| Body, button, heading, small, and monospace fonts | `Style::text_styles` |
| General gaps, margins, padding, and hit sizes | `Style::spacing` |

Use named `egui::TextStyle::Name` variants for any genuine typography role not
covered by egui's standard text styles. Normal call sites should not set the
theme's standard font sizes through `RichText::size`.

Install only the resolved style slot and then select it:

```rust
pub(crate) fn install_resolved_theme(ctx: &egui::Context, theme: &ResolvedTheme) {
    ctx.set_style_of(theme.scheme, theme.style.clone());
    ctx.set_theme(theme.scheme);
}
```

Writing the same style into both egui theme slots is unnecessary when the
application pins the selected dark or light scheme. Theme switching tests must
verify this ordering before the old two-slot workaround is removed.

## Pertylizer-specific visuals and metrics

Keep only visual concepts that egui cannot model:

```rust
#[derive(Clone, Copy, Debug)]
pub(crate) struct SynthVisuals {
    pub signals: SignalPalette,
    pub meters: MeterPalette,
    pub graph: GraphPalette,
    pub success: Color32,
    pub info: Color32,
}
```

Generic surfaces, text, selection, warnings, errors, strokes, and widget states
must be read from `ui.visuals()` or `ui.style()`, including inside custom
painters. `SynthVisuals` should not duplicate those values merely for
convenience.

Separate custom-widget geometry and interaction tuning from the selected
colour theme:

```rust
#[derive(Clone, Copy, Debug)]
pub(crate) struct SynthMetrics {
    pub knob_size: f32,
    pub small_knob_size: f32,
    pub port_column_width: f32,
    pub port_vertical_spacing: f32,
    pub cable_thickness: f32,
    pub knob_sensitivity: f32,
    pub meter_segment_gap: f32,
    pub meter_segments: usize,
}
```

This type is illustrative, not a requirement to retain every listed field.
Audit each current field and delete unused values. Keep an independent metric
only when an implemented custom widget consumes it. If compact or large UI
density is later needed, introduce a separate density setting that resolves to
metrics; do not overload the colour theme with layout policy.

## UI-thread ownership

`SynthApp` should own the selected theme and its resolved Pertylizer-specific
visuals:

```rust
pub(crate) struct ThemeState {
    selected: ThemeId,
    synth_visuals: SynthVisuals,
}

impl ThemeState {
    pub(crate) fn select(
        &mut self,
        ctx: &egui::Context,
        definition: &ThemeDefinition,
    ) {
        let resolved = definition.resolve();
        install_resolved_theme(ctx, &resolved);
        self.selected = definition.id.clone();
        self.synth_visuals = resolved.synth_visuals;
    }
}
```

All changes go through `ThemeState::select`, so egui and the synth-specific
snapshot cannot be updated separately. The state lives only on the UI thread;
it requires no lock and must never be shared with the audio engine.

Top-level views should copy or borrow one `SynthVisuals` snapshot and pass it to
custom drawing functions:

```rust
fn draw_meter(
    ui: &mut egui::Ui,
    level: MeterLevel,
    visuals: &SynthVisuals,
    metrics: &SynthMetrics,
) {
    // Custom painting only.
}
```

Do not replace the global `theme()` lookup with another hidden global or with
repeated context-data lookups. Explicit arguments make dependencies testable
and ensure inner painting loops do not acquire a lock or perform registry work.

## Shared controls

The shared controls in `widgets/controls.rs` remain the boundary for repeated
semantic UI idioms. Add or retain helpers such as primary, destructive,
success, warning, dim-label, and section-heading controls there. Their generic
parts should read `ui.style()` and `ui.visuals()`; only roles absent from egui,
such as success or signal type, receive `SynthVisuals`.

View code should express intent:

```rust
destructive_button(ui, "Delete");
signal_label(ui, visuals.signals.audio, "Audio");
```

It should not rebuild buttons with a hue-named palette field at every call
site.

## File organization

Split the current large theme module by responsibility:

```text
gui/theme/
  mod.rs          Public(crate) theme surface and shared types
  palette.rs      Semantic palette and egui style resolution
  builtins.rs     The eight built-in theme definitions
  registry.rs     ThemeId lookup and fallback policy
```

Keep `ThemeState` with the GUI application state if that avoids a dependency
from the theme module back into `egui_backend`. Do not split small files merely
to mirror each struct; the boundary above is by responsibility.

## Migration plan

### Phase 1: establish one-way resolution

1. Add `ThemeId`, `ThemeDefinition`, semantic palette types, and the built-in
   registry without changing rendered output.
2. Add the pure `ThemeDefinition::resolve` path starting from
   `egui::Theme::default_style`.
3. Map every currently installed egui field in one place, including dark/light
   scheme selection and window chrome behaviour.
4. Add tests proving that resolution is deterministic and independent of the
   previously selected preset.

During this phase the old global theme may remain as a temporary adapter, but
new code must not call it.

### Phase 2: move generic styling into egui

1. Configure standard and custom text styles centrally.
2. Configure generic spacing, margins, padding, hit size, slider rail, strokes,
   and corner radii centrally.
3. Convert generic labels, frames, buttons, text edits, sliders, and panels to
   consume `ui.style()` or `ui.visuals()`.
4. Consolidate repeated semantic controls in `widgets/controls.rs`.
5. Remove font, spacing, colour, and style values from the old theme as their
   last direct consumers disappear.

### Phase 3: make custom visuals explicit

1. Introduce the minimal `SynthVisuals` and `SynthMetrics` snapshots.
2. Convert meters, knobs, envelopes, scopes, graph canvases, ports, cables,
   keyboards, piano rolls, and trackers one view at a time.
3. Collect a snapshot once at each view boundary and pass it through custom
   painting helpers.
4. Replace hue-oriented names with semantic roles while preserving each
   preset's current colours.
5. Derive duplicated port/cable or status/meter colours where their semantics
   are intentionally identical.

### Phase 4: remove global theme state

1. Add `ThemeState` to `SynthApp` and route startup and settings-dialog changes
   through `ThemeState::select`.
2. Persist `ThemeId` in `AppSettings` and implement unknown-ID fallback.
3. Remove `THEME`, `theme()`, `try_theme()`, `set_theme()`, and
   `with_theme_mut()` after their final consumers are gone.
4. Remove unused theme fields and temporary compatibility adapters.
5. Confirm no theme lock or registry lookup occurs inside per-cell or
   per-segment drawing loops.

## Tests and verification

### Unit tests

- Every built-in ID is unique and resolves successfully.
- Every built-in definition has the expected dark or light scheme.
- Resolving preset B after preset A produces the same style as resolving B from
  a fresh egui default; no visual state leaks between presets.
- The semantic-to-egui mapping covers text, surfaces, widget states, selection,
  warning/error colours, strokes, corner radii, text styles, and spacing.
- An unknown persisted `ThemeId` selects the documented fallback and reports a
  recoverable settings warning.
- Palette validation catches unusable primary-text/background contrast and
  accidental duplicate IDs.

### GUI verification

- Switch through all eight themes on both dark- and light-mode desktops.
- Confirm application content and native window decorations agree on dark or
  light appearance.
- Check normal, hovered, active, disabled, selected, focused, and open-menu
  states.
- Check dialogs, popups, tooltips, text edits, code/script editors, hyperlinks,
  and striped grids for inherited egui defaults that previously leaked from a
  different theme.
- Check custom meters, ports, cables, scopes, envelopes, keyboards, graph
  canvases, piano rolls, and trackers against the pre-migration appearance.
- Change theme while every major view is open and confirm the update is
  immediate and complete.

### Repository quality gate

Before committing the implementation, run the required workspace-wide checks:

```bash
cargo fmt --check
cargo build
cargo clippy --workspace --all-targets
cargo test --workspace
```

## Exit gate

- Each built-in preset is defined once and preserves its current visual intent.
- Generic widgets obtain colours, typography, spacing, strokes, and state
  styling from egui rather than from Pertylizer's own theme API.
- Pertylizer-specific painters receive explicit `SynthVisuals` and
  `SynthMetrics` inputs.
- Theme changes go through one UI-thread-owned state transition that installs
  the egui style and updates the synth-specific snapshot together.
- No global theme lock or hidden replacement remains.
- No hue-named accent token remains at a semantic call site.
- No unused theme size or style field remains.
- Settings persist a stable theme identity and recover cleanly from an unknown
  ID.
- All workspace quality checks pass with zero warnings.
