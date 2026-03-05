//! Effect rack — switchable visual layers with fade-through-black crossfade.
//!
//! Keyboard controls:
//! - Left/Right arrows: previous/next effect
//! - R: random effect
//!
//! The `EffectState.fade` multiplier is read by each effect's update system
//! to scale emissive intensity, producing a smooth crossfade.

use bevy::prelude::*;

/// Available visual effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EffectId {
    #[default]
    FftBars,
    WaveformRing,
    SpectralWaterfall,
}

impl EffectId {
    const ALL: [Self; 3] = [Self::FftBars, Self::WaveformRing, Self::SpectralWaterfall];

    fn index(self) -> usize {
        match self {
            Self::FftBars => 0,
            Self::WaveformRing => 1,
            Self::SpectralWaterfall => 2,
        }
    }

    fn from_index(i: usize) -> Self {
        Self::ALL[i % Self::ALL.len()]
    }

    fn next(self) -> Self {
        Self::from_index(self.index() + 1)
    }

    fn prev(self) -> Self {
        let i = self.index();
        Self::from_index(if i == 0 { Self::ALL.len() - 1 } else { i - 1 })
    }
}

/// Marker component linking an entity to a specific effect layer.
#[derive(Component)]
pub struct EffectLayer(pub EffectId);

/// Global effect state — controls which effect is active and crossfade progress.
#[derive(Resource)]
pub struct EffectState {
    /// Currently visible effect.
    pub active: EffectId,
    /// Effect to switch to after fade-out completes.
    pending: Option<EffectId>,
    /// Fade multiplier: 1.0 = fully visible, 0.0 = black.
    pub fade: f32,
    /// True while fading out (toward 0), false while fading in (toward 1).
    fading_out: bool,
}

/// Crossfade speed (units per second).
const FADE_SPEED: f32 = 4.0;

impl Default for EffectState {
    fn default() -> Self {
        Self {
            active: EffectId::default(),
            pending: None,
            fade: 1.0,
            fading_out: false,
        }
    }
}

/// Handle keyboard input for effect switching.
pub fn input(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<EffectState>) {
    // Ignore input during crossfade
    if state.pending.is_some() {
        return;
    }

    let next = if keys.just_pressed(KeyCode::ArrowRight) {
        Some(state.active.next())
    } else if keys.just_pressed(KeyCode::ArrowLeft) {
        Some(state.active.prev())
    } else if keys.just_pressed(KeyCode::KeyR) {
        // Pick a different random effect using frame count as seed
        let current = state.active.index();
        let offset = 1
            + (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos() as usize
                % (EffectId::ALL.len() - 1));
        Some(EffectId::from_index(
            (current + offset) % EffectId::ALL.len(),
        ))
    } else {
        None
    };

    if let Some(target) = next {
        state.pending = Some(target);
        state.fading_out = true;
    }
}

/// Update crossfade: fade out → swap visibility → fade in.
pub fn crossfade(
    time: Res<Time>,
    mut state: ResMut<EffectState>,
    mut query: Query<(&EffectLayer, &mut Visibility)>,
) {
    let dt = time.delta_secs();

    if state.fading_out {
        state.fade = (state.fade - FADE_SPEED * dt).max(0.0);
        if state.fade <= 0.0 {
            // Swap: hide old, show new
            if let Some(new_effect) = state.pending.take() {
                let old_effect = state.active;
                state.active = new_effect;

                for (layer, mut vis) in &mut query {
                    if layer.0 == old_effect {
                        *vis = Visibility::Hidden;
                    } else if layer.0 == new_effect {
                        *vis = Visibility::Inherited;
                    }
                }
            }
            state.fading_out = false;
        }
    } else if state.fade < 1.0 {
        state.fade = (state.fade + FADE_SPEED * dt).min(1.0);
    }
}
