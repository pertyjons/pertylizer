//! Debug HUD overlay — toggled with F12.
//!
//! Shows diagnostic telemetry information as a semi-transparent
//! text overlay in the top-left corner of the screen.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

use super::effects::EffectState;
use super::theme::ThemeState;
use crate::telemetry::SynthTelemetry;

/// Resource tracking debug HUD visibility.
#[derive(Resource, Default)]
pub struct DebugHudState {
    pub visible: bool,
}

/// Marker for the debug HUD root node.
#[derive(Component)]
pub(crate) struct DebugHudRoot;

/// Marker for the debug HUD text entity.
#[derive(Component)]
pub(crate) struct DebugHudText;

/// Spawn the HUD UI nodes (initially hidden).
pub fn setup(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(10.0),
                left: Val::Px(10.0),
                padding: UiRect::all(Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
            Visibility::Hidden,
            DebugHudRoot,
        ))
        .with_child((
            Text::new(""),
            TextFont {
                font_size: 14.0,
                ..default()
            },
            TextColor(Color::srgba(0.0, 1.0, 0.6, 0.95)),
            DebugHudText,
        ));
}

/// Toggle HUD visibility on F12.
pub fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<DebugHudState>,
    mut query: Query<&mut Visibility, With<DebugHudRoot>>,
) {
    if keys.just_pressed(KeyCode::F12) {
        state.visible = !state.visible;
        for mut vis in &mut query {
            *vis = if state.visible {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
    }
}

/// Update HUD text each frame when visible.
pub fn update(
    state: Res<DebugHudState>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    theme_state: Option<Res<ThemeState>>,
    diagnostics: Res<DiagnosticsStore>,
    mut query: Query<&mut Text, With<DebugHudText>>,
) {
    if !state.visible {
        return;
    }

    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);

    let frame_time_ms = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);

    let theme_name = theme_state
        .as_ref()
        .map_or_else(|| "-".to_string(), |ts| format!("{:?}", ts.active));

    let scene = &effect_state.active;
    let terrain_name = scene
        .terrain
        .map_or_else(|| "-".to_string(), |e| format!("{e:?}"));
    let hero_name = scene
        .hero
        .map_or_else(|| "-".to_string(), |e| format!("{e:?}"));
    let ambient_name = scene
        .ambient
        .map_or_else(|| "-".to_string(), |e| format!("{e:?}"));
    let transients_name = if scene.transients.is_empty() {
        "-".to_string()
    } else {
        scene
            .transients
            .iter()
            .map(|e| format!("{e:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let content = format!(
        "Pertylizer Visualizer\n\
         Theme: {theme_name}\n\
         Terrain: {terrain_name}\n\
         Hero: {hero_name}  Ambient: {ambient_name}\n\
         Transients: {transients_name}\n\
         FPS: {fps:.1}  Frame: {frame_time_ms:.1}ms\n\
         -----------------------\n\
         RMS: {rms_l:.2} / {rms_r:.2}   Peak: {peak_l:.2} / {peak_r:.2}\n\
         Centroid: {centroid:.0} Hz   Flux: {flux:.3}\n\
         Tempo: {tempo:.1} BPM    Beat: {beat:.1}  Phase: {phase:.2}\n\
         Voices: {voices}           CPU: {cpu:.1}%\n\
         Event Drops: {drops}      Stale: {stale:.1}s\n\
         OSC Seq: {seq}      FFT Bins: {fft_bins}\n\
         Protocol: v{proto}",
        rms_l = telemetry.rms[0],
        rms_r = telemetry.rms[1],
        peak_l = telemetry.peak[0],
        peak_r = telemetry.peak[1],
        centroid = telemetry.centroid_hz,
        flux = telemetry.flux,
        tempo = telemetry.tempo,
        beat = telemetry.beat_position,
        phase = telemetry.beat_phase,
        voices = telemetry.voice_count,
        cpu = telemetry.cpu,
        drops = telemetry.event_drops,
        stale = telemetry.stale_seconds,
        seq = telemetry.seq,
        fft_bins = telemetry.fft_bin_count,
        proto = telemetry.protocol_version,
    );

    for mut text in &mut query {
        **text = content.clone();
    }
}
