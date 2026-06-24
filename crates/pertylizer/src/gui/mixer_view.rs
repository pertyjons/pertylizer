//! Mixer view (Channel-strip-C, Phase 7b).
//!
//! A dedicated console-style view: one vertical channel strip per sequencer
//! track (instrument label, sends, pan, mute/solo, volume fader, level meter),
//! one strip per return bus (inserts editor, pan, mute, volume, level meter),
//! and a master strip. Faders/pan/mute/solo/send levels are stored in the
//! `Song` and read live by the engine each block (Model C), so the view simply
//! mutates `song.write()`. Return-bus creation/removal additionally allocates/
//! frees the engine-side runtime channel via `CreateReturnBus`/`RemoveReturnBus`,
//! and return inserts are edited through the `*ReturnEffect*` commands.
//!
//! Per-channel post-fader peak levels are read from the engine
//! (`EngineHandle::channel_peak`/`return_peak`) and lightly smoothed for display.
//! Channel (instrument) inserts are edited in the Rack's patch editor — the
//! mixer offers a jump-to-Rack button — to avoid duplicating that editor; return
//! busses have no other editor, so their inserts are edited inline here.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use eframe::egui::{self, Color32, RichText};
use parking_lot::RwLock;

use synth_core::{BipolarValue, Gain, ModuleType, NormalizedValue, Param};
use synth_engine::{EngineCommand, EngineHandle, InstrumentId, ModuleId};
use synth_sequencer::{ReturnBusId, SeqInstrumentId, Song, TrackId, TrackSend};

use crate::gui::module_panel::category_color;
use crate::gui::theme::theme;

// --- Mixer view palette ---
/// Clip-warning header tint (used by the layout-measurement tests).
#[cfg(test)]
const CLIP_WARN_ORANGE: Color32 = Color32::from_rgb(200, 120, 40);
use crate::gui::widgets::{ModuleFrame, draw_module_header, level_color};

/// Width of a single channel strip, in points.
const STRIP_WIDTH: f32 = 108.0;
/// Width of a return-bus insert module (and thus the return column), in points.
/// Wider than a strip so the shared Rack parameter widgets — knobs, labelled
/// sliders, dropdowns — get the room they need.
const INSERT_WIDTH: f32 = 200.0;
/// Height of the big volume fader (and the level meter beside it), in points.
const FADER_HEIGHT: f32 = 160.0;
/// Width of the level-meter bar, in points.
const METER_WIDTH: f32 = 7.0;
/// Per-frame release factor for the smoothed meter (fast attack, slow release).
const METER_RELEASE: f32 = 0.82;

/// Effect types offered in the return-bus "add insert" picker. These all have a
/// `module_factory::create_effect` implementation.
const RETURN_FX: &[ModuleType] = &[
    ModuleType::Reverb,
    ModuleType::Delay,
    ModuleType::Chorus,
    ModuleType::Phaser,
    ModuleType::Flanger,
    ModuleType::Eq,
    ModuleType::Compressor,
    ModuleType::Distortion,
    ModuleType::Limiter,
    ModuleType::MidSide,
    ModuleType::BbdDelay,
    ModuleType::EnsembleChorus,
    ModuleType::ShimmerReverb,
];

/// UI state for the mixer view (rename buffer + smoothed meter levels).
#[derive(Default)]
pub struct MixerViewState {
    /// In-progress return-bus rename: `(id, edit buffer)`.
    editing_return_name: Option<(ReturnBusId, String)>,
    /// Smoothed meter level per channel, keyed by `InstrumentId`.
    channel_meter_smooth: HashMap<u64, f32>,
    /// Smoothed meter level per return bus, keyed by `ReturnBusId`.
    return_meter_smooth: HashMap<u64, f32>,
}

/// An action the mixer view asks the host to perform after rendering.
pub enum MixerViewAction {
    /// Open the Rack patch editor focused on this track's instrument (to edit
    /// the channel's insert effects, which live on the instrument).
    EditChannelFx(SeqInstrumentId),
}

/// A track channel strip, snapshotted from the song under a short read lock.
struct ChannelSnapshot {
    id: TrackId,
    name: String,
    color: Color32,
    instrument: SeqInstrumentId,
    volume: f32,
    pan: f32,
    mute: bool,
    solo: bool,
    /// Per return bus: (id, send level (0.0 = no send), pre-fader flag, enabled flag).
    sends: Vec<(ReturnBusId, f32, bool, bool)>,
}

/// A return-bus strip snapshot.
struct ReturnSnapshot {
    id: ReturnBusId,
    name: String,
    color: Color32,
    volume: f32,
    pan: f32,
    mute: bool,
    solo: bool,
    /// Bus-to-bus sends from this return: (target, level, enabled).
    sends: Vec<(ReturnBusId, f32, bool)>,
}

/// One insert effect on a return bus, cloned from the engine snapshot for
/// lock-free rendering.
#[derive(Clone)]
struct EffectInfo {
    module_id: ModuleId,
    module_type: ModuleType,
    params: Vec<Param>,
    bypassed: bool,
}

/// Everything the mixer needs for one frame.
struct MixerSnapshot {
    channels: Vec<ChannelSnapshot>,
    returns: Vec<ReturnSnapshot>,
    return_ids: Vec<(ReturnBusId, String)>,
}

/// Deferred mutations that change the set of return busses (applied after the
/// render pass to avoid touching the song mid-iteration).
enum MixerMutation {
    CreateReturn,
    DeleteReturn(ReturnBusId),
}

/// Where an inline insert-effect editor writes: a specific return bus, or the
/// master bus. Lets [`draw_effect_module`] / [`add_effect`] build the right
/// engine command (the `*ReturnEffect*` commands vs the master `*Effect*`
/// commands, which use `instrument_id: None`).
#[derive(Clone, Copy)]
enum EffectTarget {
    Return(ReturnBusId),
    Master,
}

impl EffectTarget {
    /// A widget-id seed unique per target (master is distinct from any return id).
    fn id_seed(self) -> u32 {
        match self {
            Self::Return(id) => u32::from(id.0),
            Self::Master => u32::from(u16::MAX) + 1,
        }
    }

    fn set_enabled(self, module_id: ModuleId, enabled: bool) -> EngineCommand {
        match self {
            Self::Return(return_id) => EngineCommand::SetReturnEffectEnabled {
                return_id,
                module_id,
                enabled,
            },
            Self::Master => EngineCommand::SetEffectEnabled {
                instrument_id: None,
                module_id,
                enabled,
            },
        }
    }

    fn remove(self, module_id: ModuleId) -> EngineCommand {
        match self {
            Self::Return(return_id) => EngineCommand::RemoveReturnEffect {
                return_id,
                id: module_id,
            },
            Self::Master => EngineCommand::RemoveEffect {
                instrument_id: None,
                id: module_id,
            },
        }
    }

    fn set_param(self, module_id: ModuleId, param: Param) -> EngineCommand {
        match self {
            Self::Return(return_id) => EngineCommand::SetReturnEffectParameter {
                return_id,
                module_id,
                param,
            },
            Self::Master => EngineCommand::SetEffectParameter {
                instrument_id: None,
                module_id,
                param,
            },
        }
    }

    fn add(self, id: ModuleId, effect: Box<dyn synth_core::AudioEffect>) -> EngineCommand {
        match self {
            Self::Return(return_id) => EngineCommand::AddReturnEffect {
                return_id,
                id,
                effect,
            },
            Self::Master => EngineCommand::AddEffectInstance {
                instrument_id: None,
                id,
                effect,
            },
        }
    }
}

/// Draw the mixer view. Returns an action for the host to handle (e.g. jumping
/// to the Rack to edit a channel's inserts).
pub fn draw_mixer_view(
    ui: &mut egui::Ui,
    handle: &mut EngineHandle,
    song: &Arc<RwLock<Song>>,
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
    state: &mut MixerViewState,
) -> Option<MixerViewAction> {
    let t = theme();

    // Meters animate while this view is visible. ~30 fps is plenty for level
    // ballistics and avoids pinning the whole app at the display refresh rate.
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(33));

    let Some(snapshot) = collect_snapshot(song) else {
        // Audio thread holds the write lock this frame; skip silently.
        return None;
    };

    // Clone the return-effect chains out of shared state so the inserts editor
    // can read them without holding the lock while we mutably borrow `handle`.
    let return_effects: HashMap<ReturnBusId, Vec<EffectInfo>> = {
        let guard = handle.state.return_bus_effects.read();
        guard
            .iter()
            .map(|bus| {
                let fx = bus
                    .effects
                    .iter()
                    .map(|e| EffectInfo {
                        module_id: e.module_id,
                        module_type: e.module_type,
                        params: e.parameters.clone(),
                        bypassed: e.bypassed,
                    })
                    .collect();
                (bus.id, fx)
            })
            .collect()
    };

    // Master effect chain, cloned out of shared state for the master strip.
    let master_effects: Vec<EffectInfo> = handle
        .state
        .master_effects
        .read()
        .iter()
        .map(|e| EffectInfo {
            module_id: e.module_id,
            module_type: e.module_type,
            params: e.parameters.clone(),
            bypassed: e.bypassed,
        })
        .collect();

    let mut mutation: Option<MixerMutation> = None;
    let mut action: Option<MixerViewAction> = None;

    // Fill the view background with the theme's panel color — this view draws
    // straight into the frame's central area, which is otherwise unfilled.
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(t.colors.bg_panel))
        .show_inside(ui, |ui| {
            // ── Toolbar ──
            super::toolbar::top(ui, "mixer_toolbar", |ui| {
                ui.label(RichText::new("Mixer").color(t.colors.text_primary).strong());
                ui.add_space(t.spacing.lg);
                if ui
                    .button(RichText::new("+ Return Bus").color(t.colors.accent_green))
                    .on_hover_text("Create a new effect-send return bus")
                    .clicked()
                {
                    mutation = Some(MixerMutation::CreateReturn);
                }
            });

            egui::ScrollArea::horizontal()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        for ch in &snapshot.channels {
                            let eng_id = instruments
                                .iter()
                                .find(|i| i.id.0 == u64::from(ch.instrument.0))
                                .map(|i| i.id);
                            if draw_channel_strip(
                                ui,
                                ch,
                                &snapshot.return_ids,
                                song,
                                handle,
                                eng_id,
                                state,
                            ) {
                                action = Some(MixerViewAction::EditChannelFx(ch.instrument));
                            }
                        }

                        if !snapshot.returns.is_empty() {
                            ui.separator();
                        }

                        for rb in &snapshot.returns {
                            let fx = return_effects.get(&rb.id).cloned().unwrap_or_default();
                            if draw_return_strip(
                                ui,
                                rb,
                                song,
                                handle,
                                &fx,
                                &snapshot.return_ids,
                                state,
                            ) {
                                mutation = Some(MixerMutation::DeleteReturn(rb.id));
                            }
                        }

                        ui.separator();

                        draw_master_strip(ui, handle, &master_effects);
                    });
                });
        });

    if let Some(mutation) = mutation {
        apply_mutation(mutation, handle, song);
    }
    action
}

/// Collect a render snapshot under a short read lock (released on return).
fn collect_snapshot(song: &Arc<RwLock<Song>>) -> Option<MixerSnapshot> {
    let song = song.try_read()?;

    let return_ids: Vec<(ReturnBusId, String)> = song
        .return_busses()
        .iter()
        .map(|b| (b.id, b.name.clone()))
        .collect();

    let channels = song
        .tracks()
        .map(|tr| {
            let sends = return_ids
                .iter()
                .map(|(rid, _)| {
                    tr.sends
                        .iter()
                        .find(|s| s.target == *rid)
                        .map_or((*rid, 0.0, false, true), |s| {
                            (*rid, s.level.as_f32(), s.pre_fader, s.enabled)
                        })
                })
                .collect();
            ChannelSnapshot {
                id: tr.id,
                name: tr.name.clone(),
                color: crate::gui::sequencer::track_color_to_egui(tr.color),
                instrument: tr.instrument,
                volume: tr.volume.as_f32(),
                pan: tr.pan.as_f32(),
                mute: tr.mute,
                solo: tr.solo,
                sends,
            }
        })
        .collect();

    let returns = song
        .return_busses()
        .iter()
        .map(|b| ReturnSnapshot {
            id: b.id,
            name: b.name.clone(),
            color: crate::gui::sequencer::track_color_to_egui(b.color),
            volume: b.volume.as_f32(),
            pan: b.pan.as_f32(),
            mute: b.mute,
            solo: b.solo,
            sends: b
                .sends
                .iter()
                .map(|s| (s.target, s.level.as_f32(), s.enabled))
                .collect(),
        })
        .collect();

    Some(MixerSnapshot {
        channels,
        returns,
        return_ids,
    })
}

/// Update and return the smoothed meter level for `key` toward `target`
/// (instant attack, gradual release).
fn smoothed(map: &mut HashMap<u64, f32>, key: u64, target: f32) -> f32 {
    let target = target.clamp(0.0, 1.0);
    let v = map.entry(key).or_insert(0.0);
    *v = if target > *v {
        target
    } else {
        *v * METER_RELEASE
    };
    *v
}

/// Draw a thin vertical post-fader level meter.
fn draw_meter_bar(ui: &mut egui::Ui, level: f32) {
    let t = theme();
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(METER_WIDTH, FADER_HEIGHT), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 1.0, t.colors.bg_dark);
    let h = rect.height() * level.clamp(0.0, 1.0);
    if h > 0.5 {
        let fill = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - h),
            rect.right_bottom(),
        );
        painter.rect_filled(fill, 1.0, level_color(level, &t.colors));
    }
}

/// A tall vertical fader. Returns the slider response.
fn vertical_fader(ui: &mut egui::Ui, value: &mut f32) -> egui::Response {
    // For a vertical slider egui uses `slider_width` as the rail length.
    let prev = ui.spacing().slider_width;
    ui.spacing_mut().slider_width = FADER_HEIGHT;
    let resp = ui.add(
        egui::Slider::new(value, 0.0..=1.0)
            .vertical()
            .show_value(false),
    );
    ui.spacing_mut().slider_width = prev;
    resp
}

/// Mute toggle for a strip header — frameless icon button styled like the
/// patch module's "Active" toggle, but with a red cue when muted. Returns true
/// if clicked.
fn header_mute_button(ui: &mut egui::Ui, muted: bool) -> bool {
    use egui_remixicon::icons as ri;
    let t = theme();
    let (icon, color, tip) = if muted {
        (
            ri::VOLUME_MUTE_FILL,
            t.colors.accent_red,
            "Muted\nChannel output is silenced.\nClick to unmute.",
        )
    } else {
        (
            ri::VOLUME_UP_FILL,
            t.colors.text_secondary,
            "Audible\nChannel is playing.\nClick to mute.",
        )
    };
    ui.add(
        egui::Button::new(RichText::new(icon).color(color).size(14.0))
            .frame(false)
            .min_size(egui::vec2(20.0, 20.0)),
    )
    .on_hover_text(tip)
    .clicked()
}

/// Solo toggle for a strip header — frameless headphone icon, the DAW
/// convention for solo. Returns true if clicked.
fn header_solo_button(ui: &mut egui::Ui, soloed: bool) -> bool {
    use egui_remixicon::icons as ri;
    let t = theme();
    let (icon, color, tip) = if soloed {
        (
            ri::HEADPHONE_FILL,
            t.colors.accent_yellow,
            "Soloed\nOnly soloed channels are heard.\nClick to unsolo.",
        )
    } else {
        (
            ri::HEADPHONE_LINE,
            t.colors.text_secondary,
            "Solo\nIsolate this channel.\nClick to solo.",
        )
    };
    ui.add(
        egui::Button::new(RichText::new(icon).color(color).size(14.0))
            .frame(false)
            .min_size(egui::vec2(20.0, 20.0)),
    )
    .on_hover_text(tip)
    .clicked()
}

/// The styled frame shared by every strip — the same rounded, accent-tinted
/// module look as the patch editor (via [`ModuleFrame`]). `accent` drives the
/// tint and stroke; `base_fill` is the strip's background tier (track / return
/// / master). Keeps the strip's tight 6 pt inner margin so `center_pad` stays
/// accurate.
fn strip_frame(ui: &egui::Ui, accent: Color32, base_fill: Color32) -> egui::Frame {
    ModuleFrame::new(accent)
        .base_fill(base_fill)
        .inner_margin(6.0)
        .build(&ui.global_style())
}

/// Leading space to centre `content_width` points of controls within a strip
/// (the `16.0` covers the strip's inner margin plus item spacing).
fn center_pad(content_width: f32) -> f32 {
    ((STRIP_WIDTH - content_width - 16.0) * 0.5).max(0.0)
}

/// Descriptor for an effect type, cached per type. `create_effect` builds (and
/// immediately drops) a full DSP object just to read its descriptor, so caching
/// avoids that allocation on every frame an insert is on screen. Returned as an
/// `Rc` so the per-frame, per-effect lookup is a refcount bump rather than a deep
/// clone of the descriptor's parameter vector. The GUI is single-threaded, so a
/// thread-local cache suffices.
fn cached_descriptor(module_type: ModuleType) -> Option<Rc<synth_core::ModuleDescriptor>> {
    thread_local! {
        static CACHE: std::cell::RefCell<HashMap<ModuleType, Rc<synth_core::ModuleDescriptor>>> =
            std::cell::RefCell::new(HashMap::new());
    }
    CACHE.with(|cache| {
        if let Some(d) = cache.borrow().get(&module_type) {
            return Some(Rc::clone(d));
        }
        let (_, descriptor) = crate::module_factory::create_effect(module_type)?;
        let descriptor = Rc::new(descriptor);
        cache
            .borrow_mut()
            .insert(module_type, Rc::clone(&descriptor));
        Some(descriptor)
    })
}

/// Draw one track channel strip; mutations are written straight to the song.
/// Returns true if the user clicked the jump-to-Rack inserts button.
#[allow(clippy::too_many_arguments)]
fn draw_channel_strip(
    ui: &mut egui::Ui,
    ch: &ChannelSnapshot,
    return_ids: &[(ReturnBusId, String)],
    song: &Arc<RwLock<Song>>,
    handle: &EngineHandle,
    eng_id: Option<InstrumentId>,
    state: &mut MixerViewState,
) -> bool {
    use egui_remixicon::icons as ri;

    let t = theme();
    let mut edit_fx = false;
    ui.allocate_ui(egui::vec2(STRIP_WIDTH, 0.0), |ui| {
        ui.set_width(STRIP_WIDTH);
        strip_frame(ui, ch.color, t.colors.bg_module).show(ui, |ui| {
            // The frame inherits the horizontal_top layout of the strip row, so
            // wrap everything in a vertical layout to stack the header above the
            // controls (and give the column the full strip width).
            ui.vertical(|ui| {
                // Channel name in the accent-tinted module header, with the
                // mute/solo toggles living in the header bar the same way the
                // patch-module "Active" toggle does.
                draw_module_header(ui, ch.color, &ch.name, None, |ui| {
                    if header_mute_button(ui, ch.mute)
                        && let Some(tr) = song.write().track_mut(ch.id)
                    {
                        tr.toggle_mute();
                    }
                    if header_solo_button(ui, ch.solo)
                        && let Some(tr) = song.write().track_mut(ch.id)
                    {
                        tr.toggle_solo();
                    }
                    // Inserts live on the instrument and are edited in the Rack.
                    // The external-link icon signals the button jumps to another
                    // view rather than opening an inline editor.
                    ui.separator();
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(ri::EXTERNAL_LINK_LINE)
                                    .color(t.colors.text_secondary)
                                    .size(14.0),
                            )
                            .frame(false)
                            .min_size(egui::vec2(20.0, 20.0)),
                        )
                        .on_hover_text("Edit this channel's insert effects in the Rack")
                        .clicked()
                    {
                        edit_fx = true;
                    }
                });

                // Sends to each return bus.
                if !return_ids.is_empty() {
                    ui.label(
                        RichText::new("Sends")
                            .size(t.fonts.size_small)
                            .color(t.colors.text_secondary),
                    );
                    for (rid, rname) in return_ids {
                        let (_, level, pre, enabled) = ch
                            .sends
                            .iter()
                            .find(|(id, _, _, _)| id == rid)
                            .copied()
                            .unwrap_or((*rid, 0.0, false, true));
                        draw_send_row(ui, song, ch.id, *rid, rname, level, pre, enabled);
                    }
                    ui.add_space(t.spacing.xs);
                }

                // Pan.
                let mut pan = ch.pan;
                if ui
                    .add(
                        egui::Slider::new(&mut pan, -1.0..=1.0)
                            .show_value(true)
                            .fixed_decimals(2)
                            .text("Pan"),
                    )
                    .changed()
                    && let Some(tr) = song.write().track_mut(ch.id)
                {
                    tr.pan = BipolarValue::new(pan);
                }

                // Level meter + volume fader, side by side.
                let peak = eng_id.map_or(0.0, |id| handle.channel_peak(id));
                let level = eng_id
                    .map(|id| smoothed(&mut state.channel_meter_smooth, id.0, peak))
                    .unwrap_or(0.0);
                ui.horizontal(|ui| {
                    ui.add_space(center_pad(METER_WIDTH + 24.0));
                    draw_meter_bar(ui, level);
                    let mut vol = ch.volume;
                    if vertical_fader(ui, &mut vol).changed()
                        && let Some(tr) = song.write().track_mut(ch.id)
                    {
                        tr.volume = NormalizedValue::new(vol);
                    }
                });
                ui.label(
                    RichText::new(format!("{:.2}", ch.volume))
                        .size(t.fonts.size_small)
                        .color(t.colors.text_secondary),
                );
            });
        });
    });
    edit_fx
}

/// One send row: a level slider plus a pre/post toggle. A level of 0 removes the
/// send entirely; any positive level creates/updates it.
#[allow(clippy::too_many_arguments)]
fn draw_send_row(
    ui: &mut egui::Ui,
    song: &Arc<RwLock<Song>>,
    track: TrackId,
    target: ReturnBusId,
    name: &str,
    level: f32,
    pre_fader: bool,
    enabled: bool,
) {
    let t = theme();
    let active = level > 0.0;
    ui.horizontal(|ui| {
        // Enable / bypass toggle: a non-destructive on/off for an active send
        // (keeps its level & tap point). Only meaningful once a send exists.
        let mut on = enabled;
        if ui
            .add_enabled(active, egui::Checkbox::without_text(&mut on))
            .on_hover_text("Enable / bypass this send")
            .changed()
        {
            set_send_enabled(song, track, target, on);
        }
        let short: String = name.chars().take(4).collect();
        // Bypassed (active level but disabled) reads in the muted accent so it's
        // visually distinct from a plain inactive/enabled send.
        let label_color = if active && !enabled {
            t.colors.accent_orange
        } else {
            t.colors.text_dim
        };
        ui.label(
            RichText::new(short)
                .size(t.fonts.size_small)
                .color(label_color),
        )
        .on_hover_text(name);
        let mut lvl = level;
        if ui
            .add(
                egui::Slider::new(&mut lvl, 0.0..=1.0)
                    .show_value(false)
                    .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 0.5 }),
            )
            .on_hover_text(format!("Send to {name}"))
            .changed()
        {
            apply_send(song, track, target, lvl, pre_fader);
        }
        // Pre/post toggle (only meaningful when the send is active).
        let pp = if pre_fader { "Pre" } else { "Post" };
        if ui
            .add_enabled(
                active,
                egui::Button::new(
                    RichText::new(pp)
                        .size(t.fonts.size_small)
                        .color(t.colors.text_secondary),
                ),
            )
            .on_hover_text("Toggle pre/post-fader send")
            .clicked()
        {
            apply_send(song, track, target, level, !pre_fader);
        }
    });
}

/// Toggle a send's non-destructive enable flag. No-op if the send doesn't exist.
fn set_send_enabled(song: &Arc<RwLock<Song>>, track: TrackId, target: ReturnBusId, enabled: bool) {
    let mut sw = song.write();
    if let Some(tr) = sw.track_mut(track)
        && let Some(send) = tr.sends.iter_mut().find(|s| s.target == target)
    {
        send.enabled = enabled;
    }
}

/// Draw bus-to-bus send rows for a return strip: one per *other* return bus this
/// return can legally feed (no self, no cycle), each with a level slider and an
/// enable toggle. Level 0 removes the send. Cyclic targets are hidden.
fn draw_return_sends(
    ui: &mut egui::Ui,
    song: &Arc<RwLock<Song>>,
    rb: &ReturnSnapshot,
    return_ids: &[(ReturnBusId, String)],
) {
    let t = theme();
    let candidates: Vec<(ReturnBusId, String)> = {
        let s = song.read();
        return_ids
            .iter()
            .filter(|(id, _)| *id != rb.id && !s.return_send_would_cycle(rb.id, *id))
            .cloned()
            .collect()
    };
    if candidates.is_empty() {
        return;
    }
    ui.label(
        RichText::new("Bus sends")
            .size(t.fonts.size_small)
            .color(t.colors.text_secondary),
    );
    for (target, tname) in &candidates {
        let (level, enabled) = rb
            .sends
            .iter()
            .find(|(id, _, _)| id == target)
            .map_or((0.0, true), |(_, l, e)| (*l, *e));
        let active = level > 0.0;
        ui.horizontal(|ui| {
            let mut on = enabled;
            if ui
                .add_enabled(active, egui::Checkbox::without_text(&mut on))
                .on_hover_text("Enable / bypass this bus send")
                .changed()
            {
                set_return_send_enabled(song, rb.id, *target, on);
            }
            let short: String = tname.chars().take(4).collect();
            let color = if active && !enabled {
                t.colors.accent_orange
            } else {
                t.colors.text_dim
            };
            ui.label(RichText::new(short).size(t.fonts.size_small).color(color))
                .on_hover_text(tname);
            let mut lvl = level;
            if ui
                .add(
                    egui::Slider::new(&mut lvl, 0.0..=1.0)
                        .show_value(false)
                        .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 0.5 }),
                )
                .on_hover_text(format!("Send to {tname}"))
                .changed()
            {
                apply_return_send(song, rb.id, *target, lvl);
            }
        });
    }
}

/// Write a bus-to-bus send to the song: level 0 removes it, positive level
/// upserts it. Guards against cycles even though the UI hides cyclic targets.
fn apply_return_send(song: &Arc<RwLock<Song>>, from: ReturnBusId, target: ReturnBusId, level: f32) {
    let mut sw = song.write();
    if level <= 0.0 {
        if let Some(bus) = sw.return_bus_mut(from) {
            bus.sends.retain(|s| s.target != target);
        }
        return;
    }
    if sw.return_send_would_cycle(from, target) {
        return;
    }
    if let Some(bus) = sw.return_bus_mut(from) {
        if let Some(send) = bus.sends.iter_mut().find(|s| s.target == target) {
            send.level = NormalizedValue::new(level);
        } else {
            bus.sends.push(synth_sequencer::ReturnSend {
                target,
                level: NormalizedValue::new(level),
                enabled: true,
            });
        }
    }
}

/// Toggle a bus-to-bus send's enable flag. No-op if the send doesn't exist.
fn set_return_send_enabled(
    song: &Arc<RwLock<Song>>,
    from: ReturnBusId,
    target: ReturnBusId,
    enabled: bool,
) {
    let mut sw = song.write();
    if let Some(bus) = sw.return_bus_mut(from)
        && let Some(send) = bus.sends.iter_mut().find(|s| s.target == target)
    {
        send.enabled = enabled;
    }
}

/// Write a send to the song: level 0 removes it, positive level upserts it.
fn apply_send(
    song: &Arc<RwLock<Song>>,
    track: TrackId,
    target: ReturnBusId,
    level: f32,
    pre_fader: bool,
) {
    let mut sw = song.write();
    let Some(tr) = sw.track_mut(track) else {
        return;
    };
    if level <= 0.0 {
        tr.sends.retain(|s| s.target != target);
    } else if let Some(send) = tr.sends.iter_mut().find(|s| s.target == target) {
        send.level = NormalizedValue::new(level);
        send.pre_fader = pre_fader;
    } else {
        tr.sends.push(TrackSend {
            target,
            level: NormalizedValue::new(level),
            pre_fader,
            enabled: true,
        });
    }
}

/// Draw a return-bus strip. Returns true if the user asked to delete it.
#[allow(clippy::too_many_arguments)]
fn draw_return_strip(
    ui: &mut egui::Ui,
    rb: &ReturnSnapshot,
    song: &Arc<RwLock<Song>>,
    handle: &mut EngineHandle,
    effects: &[EffectInfo],
    return_ids: &[(ReturnBusId, String)],
    state: &mut MixerViewState,
) -> bool {
    use egui_remixicon::icons as ri;

    let t = theme();
    let mut delete = false;
    // The column is as wide as its insert modules, but collapses to the plain
    // strip width when the bus has no inserts so empty returns don't leave a
    // wide blank gap. The strip itself always keeps the standard strip width and
    // sits left-aligned at the top.
    let col_width = if effects.is_empty() {
        STRIP_WIDTH
    } else {
        INSERT_WIDTH
    };
    ui.allocate_ui(egui::vec2(col_width, 0.0), |ui| {
        ui.set_width(col_width);
        // The strip row is laid out left-to-right, so wrap the strip and its
        // insert modules in a vertical column: the strip on top, then each
        // effect stacked below it (the way the Rack stacks an effect chain) so
        // the chain is always visible without a popup.
        ui.vertical(|ui| {
            ui.allocate_ui(egui::vec2(STRIP_WIDTH, 0.0), |ui| {
                ui.set_width(STRIP_WIDTH);
                strip_frame(ui, t.colors.accent_green, t.colors.bg_panel).show(ui, |ui| {
                    // The frame inherits the horizontal_top layout of the strip row, so
                    // wrap everything in a vertical layout to stack the header above the
                    // controls (and give the column the full strip width).
                    ui.vertical(|ui| {
                        // Name — click the header to rename inline.
                        let editing = state
                            .editing_return_name
                            .as_ref()
                            .is_some_and(|(id, _)| *id == rb.id);
                        if editing {
                            if let Some((_, buf)) = state.editing_return_name.as_mut() {
                                let resp = ui.add(
                                    egui::TextEdit::singleline(buf)
                                        .desired_width(STRIP_WIDTH - 16.0)
                                        .hint_text("Name"),
                                );
                                if resp.lost_focus() {
                                    let new_name = buf.clone();
                                    if let Some(bus) = song.write().return_bus_mut(rb.id) {
                                        bus.name = new_name;
                                    }
                                    state.editing_return_name = None;
                                } else {
                                    resp.request_focus();
                                }
                            }
                        } else {
                            let resp = draw_module_header(
                                ui,
                                rb.color,
                                &format!("⮌ {}", rb.name),
                                Some("Click to rename".to_owned()),
                                |ui| {
                                    if header_mute_button(ui, rb.mute)
                                        && let Some(bus) = song.write().return_bus_mut(rb.id)
                                    {
                                        bus.set_mute(!bus.mute);
                                    }
                                    if header_solo_button(ui, rb.solo)
                                        && let Some(bus) = song.write().return_bus_mut(rb.id)
                                    {
                                        bus.set_solo(!bus.solo);
                                    }
                                    // Close button, mirroring the patch module's header.
                                    ui.separator();
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(ri::CLOSE_LINE)
                                                    .color(t.colors.text_dim)
                                                    .size(12.0),
                                            )
                                            .frame(false)
                                            .min_size(egui::vec2(20.0, 20.0)),
                                        )
                                        .on_hover_text("Delete return bus")
                                        .clicked()
                                    {
                                        delete = true;
                                    }
                                },
                            );
                            if resp.clicked() {
                                state.editing_return_name = Some((rb.id, rb.name.clone()));
                            }
                        }

                        ui.add_space(t.spacing.xs);

                        // Pan.
                        let mut pan = rb.pan;
                        if ui
                            .add(
                                egui::Slider::new(&mut pan, -1.0..=1.0)
                                    .show_value(true)
                                    .fixed_decimals(2)
                                    .text("Pan"),
                            )
                            .changed()
                            && let Some(bus) = song.write().return_bus_mut(rb.id)
                        {
                            bus.pan = BipolarValue::new(pan);
                        }

                        // Bus-to-bus sends: route this return into other returns
                        // (acyclic only). Each candidate target gets a level slider
                        // + enable toggle; cyclic targets are hidden.
                        draw_return_sends(ui, song, rb, return_ids);

                        // Level meter + volume fader.
                        let level = smoothed(
                            &mut state.return_meter_smooth,
                            u64::from(rb.id.0),
                            handle.return_peak(rb.id),
                        );
                        ui.horizontal(|ui| {
                            ui.add_space(center_pad(METER_WIDTH + 24.0));
                            draw_meter_bar(ui, level);
                            let mut vol = rb.volume;
                            if vertical_fader(ui, &mut vol).changed()
                                && let Some(bus) = song.write().return_bus_mut(rb.id)
                            {
                                bus.volume = NormalizedValue::new(vol);
                            }
                        });
                        ui.label(
                            RichText::new(format!("{:.2}", rb.volume))
                                .size(t.fonts.size_small)
                                .color(t.colors.text_secondary),
                        );
                    });
                });
            });

            // Insert effects (Rack-width modules), stacked vertically beneath the
            // strip the way the Rack stacks an effect chain. A compact menu
            // appends a new effect.
            for fx in effects {
                ui.add_space(t.spacing.xs);
                draw_effect_module(ui, EffectTarget::Return(rb.id), fx, handle);
            }
            ui.add_space(t.spacing.xs);
            ui.menu_button(
                RichText::new("+ Add FX")
                    .size(t.fonts.size_small)
                    .color(t.colors.accent_green),
                |ui| {
                    for &mt in RETURN_FX {
                        if ui.button(mt.name()).clicked() {
                            add_effect(handle, EffectTarget::Return(rb.id), mt, effects);
                            ui.close();
                        }
                    }
                },
            );
        });
    });
    delete
}

/// Draw one return-bus effect as a Rack-style module box: the shared
/// [`ModuleFrame`] tinted by the effect's category, a [`draw_module_header`]
/// carrying the bypass toggle and remove button, then the shared descriptor-
/// driven parameter grid (knobs/sliders/dropdowns/toggles — identical to the
/// patch editor). Bypassed effects are dimmed. Stacked vertically beneath the
/// return strip by the caller so the inserts read like a patch-module chain.
fn draw_effect_module(
    ui: &mut egui::Ui,
    target: EffectTarget,
    fx: &EffectInfo,
    handle: &mut EngineHandle,
) {
    let t = theme();
    let descriptor = cached_descriptor(fx.module_type);
    let accent = descriptor
        .as_ref()
        .map_or(t.colors.accent_cyan, |d| category_color(d.category));
    let frame = ModuleFrame::new(accent)
        .opacity(if fx.bypassed { 0.55 } else { 1.0 })
        .inner_margin(6.0)
        .build(&ui.global_style());
    frame.show(ui, |ui| {
        // A unique id scope per effect so two inserts of the same type on a bus
        // (and the same type across busses / master) don't collide on widget ids.
        ui.push_id((target.id_seed(), fx.module_id), |ui| {
            ui.vertical(|ui| {
                ui.set_width(INSERT_WIDTH - 12.0);
                draw_module_header(ui, accent, fx.module_type.name(), None, |ui| {
                    let mut enabled = !fx.bypassed;
                    if ui
                        .checkbox(&mut enabled, "")
                        .on_hover_text("Enable / bypass")
                        .changed()
                    {
                        handle.send(target.set_enabled(fx.module_id, enabled));
                    }
                    ui.separator();
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("✖")
                                    .size(t.fonts.size_small)
                                    .color(t.colors.text_dim),
                            )
                            .frame(false)
                            .min_size(egui::vec2(20.0, 20.0)),
                        )
                        .on_hover_text("Remove effect")
                        .clicked()
                    {
                        handle.send(target.remove(fx.module_id));
                    }
                });

                // Parameter widgets rendered by the shared, descriptor-driven
                // grid (knobs, sliders, dropdowns, toggles) — the same code the
                // Rack uses. Values come from the engine snapshot; every choice
                // is shown (no mod-matrix filtering on an effect).
                if let Some(descriptor) = descriptor.as_deref() {
                    let changes = crate::gui::widgets::draw_parameter_grid(
                        ui,
                        descriptor,
                        accent,
                        |p| {
                            fx.params
                                .iter()
                                .find(|pp| pp.same_kind(&p.id))
                                .map_or(p.range.default, Param::as_f32)
                        },
                        |_, _| true,
                        // Effects have no Mod Matrix participation in the mixer view.
                        |_| None,
                    );
                    for (param, value) in changes {
                        handle.send(target.set_param(fx.module_id, param.id.with_f32(value)));
                    }
                }
            });
        });
    });
}

/// Create and append a new effect to a return-bus or master chain. The fresh
/// `ModuleId` instance is one past the highest existing instance of that type.
fn add_effect(
    handle: &mut EngineHandle,
    target: EffectTarget,
    module_type: ModuleType,
    effects: &[EffectInfo],
) {
    let Some((effect, _descriptor)) = crate::module_factory::create_effect(module_type) else {
        return;
    };
    let instance = effects
        .iter()
        .filter(|fx| fx.module_type == module_type)
        .map(|fx| fx.module_id.instance)
        .max()
        .map_or(1, |m| m.saturating_add(1));
    handle.send(target.add(ModuleId::new(module_type, instance), effect));
}

/// Draw the master strip with its insert-effect chain. Master volume is
/// engine-owned (atomic + command); the master effects are an engine-side chain
/// (`instrument_id: None`), edited inline here like the return inserts.
fn draw_master_strip(ui: &mut egui::Ui, handle: &mut EngineHandle, effects: &[EffectInfo]) {
    let t = theme();
    let master = handle.state.master_volume.load();
    let (peak_l, peak_r) = handle.peak_meters();
    let level = peak_l.as_f32().max(peak_r.as_f32());
    ui.vertical(|ui| {
        ui.allocate_ui(egui::vec2(STRIP_WIDTH, 0.0), |ui| {
            ui.set_width(STRIP_WIDTH);
            strip_frame(ui, t.colors.accent_primary, t.colors.bg_widget).show(ui, |ui| {
                ui.vertical(|ui| {
                    draw_module_header(ui, t.colors.accent_primary, "Master", None, |_| {});
                    ui.add_space(t.spacing.xs);

                    ui.horizontal(|ui| {
                        ui.add_space(center_pad(METER_WIDTH + 24.0));
                        draw_meter_bar(ui, level);
                        let mut vol = master;
                        if vertical_fader(ui, &mut vol).changed() {
                            handle.send(EngineCommand::SetMasterVolume(Gain::new(vol)));
                        }
                    });
                    ui.label(
                        RichText::new(format!("{master:.2}"))
                            .size(t.fonts.size_small)
                            .color(t.colors.text_secondary),
                    );
                });
            });
        });

        // Master insert effects (final chain applied to the full mix), stacked
        // beneath the fader like the return inserts.
        for fx in effects {
            ui.add_space(t.spacing.xs);
            draw_effect_module(ui, EffectTarget::Master, fx, handle);
        }
        ui.add_space(t.spacing.xs);
        ui.menu_button(
            RichText::new("+ Add FX")
                .size(t.fonts.size_small)
                .color(t.colors.accent_green),
            |ui| {
                for &mt in RETURN_FX {
                    if ui.button(mt.name()).clicked() {
                        add_effect(handle, EffectTarget::Master, mt, effects);
                        ui.close();
                    }
                }
            },
        );
    });
}

/// Apply a deferred return-bus add/remove (song write + engine channel command).
fn apply_mutation(mutation: MixerMutation, handle: &mut EngineHandle, song: &Arc<RwLock<Song>>) {
    match mutation {
        MixerMutation::CreateReturn => {
            let name = {
                let n = song.read().return_busses().len() + 1;
                format!("Return {n}")
            };
            let id = song.write().create_return_bus(name);
            if !handle.send(EngineCommand::CreateReturnBus { id }) {
                // Roll back so song and engine agree on the return-bus set.
                song.write().delete_return_bus(id);
            }
        }
        MixerMutation::DeleteReturn(id) => {
            if song.write().delete_return_bus(id).is_some() {
                handle.send(EngineCommand::RemoveReturnBus { id });
            }
        }
    }
}

#[cfg(test)]
mod layout_tests {
    use super::STRIP_WIDTH;
    use crate::gui::widgets::draw_module_header;
    use eframe::egui;

    /// Measure the `available_width` of a strip's content column, reproducing the
    /// real layout nesting: the strip row is `horizontal_top`, so the frame
    /// inherits a horizontal layout. `wrapped` mirrors the strips, putting the
    /// header and column inside one `ui.vertical`; the unwrapped variant draws
    /// the header directly in the (horizontal) frame, which leaves the sibling
    /// column with ~zero width — the bug the wrapping fixes.
    fn column_available_width(wrapped: bool) -> f32 {
        let ctx = egui::Context::default();
        let mut avail = f32::NAN;
        // Two passes: the first lays out fonts/sizing, the second is stable.
        for _ in 0..2 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.allocate_ui(egui::vec2(STRIP_WIDTH, 0.0), |ui| {
                            ui.set_width(STRIP_WIDTH);
                            egui::Frame::new().inner_margin(6.0).show(ui, |ui| {
                                let mut body = |ui: &mut egui::Ui| {
                                    draw_module_header(
                                        ui,
                                        super::CLIP_WARN_ORANGE,
                                        "V1 pulse",
                                        None,
                                        |_| {},
                                    );
                                    avail = ui.available_width();
                                };
                                if wrapped {
                                    ui.vertical(body);
                                } else {
                                    body(ui);
                                }
                            });
                        });
                    });
                });
            });
        }
        avail
    }

    /// Regression guard for the channel-strip layout: drawing the module header
    /// directly in the (horizontal) frame leaves the content column with ~zero
    /// width; wrapping header + content in one `ui.vertical` stacks them and
    /// restores the full strip content width.
    #[test]
    fn wrapping_in_a_vertical_layout_stacks_header_and_content() {
        let unwrapped = column_available_width(false);
        let wrapped = column_available_width(true);
        assert!(
            unwrapped < 1.0,
            "header drawn in the horizontal frame collapses the column: got {unwrapped}"
        );
        assert!(
            wrapped > STRIP_WIDTH - 16.0,
            "wrapping in a vertical layout keeps ~the full strip width: got {wrapped}"
        );
    }
}
