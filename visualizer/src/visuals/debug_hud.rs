//! Debug HUD overlay — toggled with H.
//!
//! Shows diagnostic telemetry information as a semi-transparent
//! text overlay in the top-left corner of the screen.
//! Also handles screenshot (P) and fullscreen toggle (F).

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{MonitorSelection, WindowMode};

use super::camera::CameraState;
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

/// Spawn the HUD UI node (initially hidden).
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
            GlobalZIndex(10),
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

/// Toggle HUD visibility on H.
pub fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<DebugHudState>,
    mut query: Query<&mut Visibility, With<DebugHudRoot>>,
) {
    if keys.just_pressed(KeyCode::KeyH) {
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

/// Take a screenshot on P key.
pub fn screenshot(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut counter: Local<u32>,
) {
    if keys.just_pressed(KeyCode::KeyP) {
        let path = format!("./screenshot-{}.png", *counter);
        *counter += 1;
        info!("Saving screenshot to {path}");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    }
}

/// Toggle fullscreen on F key.
pub fn fullscreen(keys: Res<ButtonInput<KeyCode>>, mut windows: Query<&mut Window>) {
    if keys.just_pressed(KeyCode::KeyF)
        && let Ok(mut window) = windows.single_mut()
    {
        window.mode = match window.mode {
            WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            _ => WindowMode::Windowed,
        };
    }
}

/// Update HUD text each frame when visible.
pub fn update(
    state: Res<DebugHudState>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    theme_state: Option<Res<ThemeState>>,
    camera_state: Res<CameraState>,
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

    let cam_mode = camera_state.mode.name();
    let auto_cut = if camera_state.auto_cut { "ON" } else { "OFF" };

    let content = format!(
        "Pertylizer Visualizer\n\
         Theme: {theme_name}   Camera: {cam_mode}  Auto-cut: {auto_cut}\n\
         Terrain: {terrain_name}\n\
         Hero: {hero_name}  Ambient: {ambient_name}\n\
         Transients: {transients_name}\n\
         FPS:  {fps:>6.1}   Frame: {frame_time_ms:>6.1}ms\n\
         --------------------------------\n\
         RMS:  {rms_l:>5.2} / {rms_r:>5.2}   Peak:  {peak_l:>5.2} / {peak_r:>5.2}\n\
         Cent: {centroid:>8.0} Hz     Flux:  {flux:>6.3}\n\
         BPM:  {tempo:>6.1}          Beat:  {beat:>5.1}\n\
         Phase:{phase:>5.2}           Voices:{voices:>4}\n\
         CPU:  {cpu:>5.1}%          Drops: {drops:>5}\n\
         Stale:{stale:>5.1}s          Seq:   {seq:>8}\n\
         FFT:  {fft_bins:>4} bins       Proto: v{proto}\n\
         -----------------------\n\
         Shortcuts:\n\
         Left/Right  Effect prev/next\n\
         Up/Down     Zoom in/out\n\
         R           Random effect\n\
         T/Shift+T   Theme next/prev\n\
         C/Shift+C   Camera mode next/prev\n\
         V           Auto-cut toggle\n\
         F           Fullscreen toggle\n\
         P           Screenshot\n\
         H           Toggle this HUD",
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
