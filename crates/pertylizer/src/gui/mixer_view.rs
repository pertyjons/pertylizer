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
use std::sync::Arc;

use eframe::egui::{self, Color32, RichText};
use parking_lot::RwLock;

use synth_core::{BipolarValue, Gain, ModuleType, NormalizedValue, Param};
use synth_engine::{EngineCommand, EngineHandle, InstrumentId, ModuleId};
use synth_sequencer::{ReturnBusId, SeqInstrumentId, Song, TrackId, TrackSend};

use crate::gui::theme::theme;
use crate::gui::widgets::level_color;

/// Width of a single channel strip, in points.
const STRIP_WIDTH: f32 = 108.0;
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
    /// Send level (0.0 = no send) per return bus, plus pre-fader flag.
    sends: Vec<(ReturnBusId, f32, bool)>,
}

/// A return-bus strip snapshot.
struct ReturnSnapshot {
    id: ReturnBusId,
    name: String,
    volume: f32,
    pan: f32,
    mute: bool,
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

    // Clone the return-effect chains out of shared state so the popup editor can
    // read them without holding the lock while we mutably borrow `handle`.
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

    let mut mutation: Option<MixerMutation> = None;
    let mut action: Option<MixerViewAction> = None;

    // ── Toolbar ──
    ui.horizontal(|ui| {
        ui.heading(RichText::new("Mixer").color(t.colors.text_primary));
        ui.add_space(12.0);
        if ui
            .button(RichText::new("+ Return Bus").color(t.colors.accent_green))
            .on_hover_text("Create a new effect-send return bus")
            .clicked()
        {
            mutation = Some(MixerMutation::CreateReturn);
        }
    });
    ui.separator();

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
                        instruments,
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
                    if draw_return_strip(ui, rb, song, handle, &fx, state) {
                        mutation = Some(MixerMutation::DeleteReturn(rb.id));
                    }
                }

                ui.separator();

                draw_master_strip(ui, handle);
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
                        .map_or((*rid, 0.0, false), |s| {
                            (*rid, s.level.as_f32(), s.pre_fader)
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
            volume: b.volume.as_f32(),
            pan: b.pan.as_f32(),
            mute: b.mute,
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

/// A small mute/solo toggle button. Returns true if clicked.
fn toggle_chip(ui: &mut egui::Ui, label: &str, active: bool, on_color: Color32) -> bool {
    let t = theme();
    let color = if active { on_color } else { t.colors.text_dim };
    ui.add(
        egui::Button::new(RichText::new(label).size(t.fonts.size_small).color(color))
            .min_size(egui::vec2(20.0, 18.0)),
    )
    .clicked()
}

/// The styled frame shared by every strip (only fill/border colours differ).
fn strip_frame(fill: Color32, border: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, border))
        .inner_margin(6.0)
        .corner_radius(4.0)
}

/// Leading space to centre `content_width` points of controls within a strip
/// (the `16.0` covers the strip's inner margin plus item spacing).
fn center_pad(content_width: f32) -> f32 {
    ((STRIP_WIDTH - content_width - 16.0) * 0.5).max(0.0)
}

/// Descriptor for an effect type, cached per type. `create_effect` builds (and
/// immediately drops) a full DSP object just to read its descriptor, so caching
/// avoids that allocation on every frame the inserts popup is open. The GUI is
/// single-threaded, so a thread-local cache suffices.
fn cached_descriptor(module_type: ModuleType) -> Option<synth_core::ModuleDescriptor> {
    thread_local! {
        static CACHE: std::cell::RefCell<HashMap<ModuleType, synth_core::ModuleDescriptor>> =
            std::cell::RefCell::new(HashMap::new());
    }
    CACHE.with(|cache| {
        if let Some(d) = cache.borrow().get(&module_type) {
            return Some(d.clone());
        }
        let (_, descriptor) = crate::module_factory::create_effect(module_type)?;
        cache.borrow_mut().insert(module_type, descriptor.clone());
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
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
    song: &Arc<RwLock<Song>>,
    handle: &EngineHandle,
    eng_id: Option<InstrumentId>,
    state: &mut MixerViewState,
) -> bool {
    let t = theme();
    let mut edit_fx = false;
    ui.allocate_ui(egui::vec2(STRIP_WIDTH, 0.0), |ui| {
        ui.set_width(STRIP_WIDTH);
        strip_frame(t.colors.bg_module, t.colors.border).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                // Colour swatch + track name.
                let (rect, _) = ui
                    .allocate_exact_size(egui::vec2(STRIP_WIDTH - 16.0, 4.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 1.0, ch.color);
                ui.label(
                    RichText::new(&ch.name)
                        .size(t.fonts.size_small)
                        .strong()
                        .color(t.colors.text_primary),
                );

                // Instrument name (or "(none)" when id maps to nothing).
                let inst_name = instruments
                    .iter()
                    .find(|i| i.id.0 == u64::from(ch.instrument.0))
                    .map_or_else(|| "— (none) —".to_owned(), |i| i.name.clone());
                ui.label(
                    RichText::new(inst_name)
                        .size(t.fonts.size_small)
                        .color(t.colors.text_dim),
                );

                // Inserts: edited in the Rack (they live on the instrument).
                if ui
                    .add(egui::Button::new(
                        RichText::new("Inserts ▸")
                            .size(t.fonts.size_small)
                            .color(t.colors.text_secondary),
                    ))
                    .on_hover_text("Edit this channel's insert effects in the Rack")
                    .clicked()
                {
                    edit_fx = true;
                }

                ui.add_space(4.0);

                // Sends to each return bus.
                if !return_ids.is_empty() {
                    ui.label(
                        RichText::new("Sends")
                            .size(t.fonts.size_small)
                            .color(t.colors.text_secondary),
                    );
                    for (rid, rname) in return_ids {
                        let (_, level, pre) = ch
                            .sends
                            .iter()
                            .find(|(id, _, _)| id == rid)
                            .copied()
                            .unwrap_or((*rid, 0.0, false));
                        draw_send_row(ui, song, ch.id, *rid, rname, level, pre);
                    }
                    ui.add_space(4.0);
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

                // Mute / Solo.
                ui.horizontal(|ui| {
                    ui.add_space(center_pad(2.0 * 20.0));
                    if toggle_chip(ui, "M", ch.mute, t.colors.accent_red)
                        && let Some(tr) = song.write().track_mut(ch.id)
                    {
                        tr.toggle_mute();
                    }
                    if toggle_chip(ui, "S", ch.solo, t.colors.accent_yellow)
                        && let Some(tr) = song.write().track_mut(ch.id)
                    {
                        tr.toggle_solo();
                    }
                });

                ui.add_space(4.0);

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
fn draw_send_row(
    ui: &mut egui::Ui,
    song: &Arc<RwLock<Song>>,
    track: TrackId,
    target: ReturnBusId,
    name: &str,
    level: f32,
    pre_fader: bool,
) {
    let t = theme();
    ui.horizontal(|ui| {
        let short: String = name.chars().take(4).collect();
        ui.label(
            RichText::new(short)
                .size(t.fonts.size_small)
                .color(t.colors.text_dim),
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
                level > 0.0,
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
        });
    }
}

/// Draw a return-bus strip. Returns true if the user asked to delete it.
fn draw_return_strip(
    ui: &mut egui::Ui,
    rb: &ReturnSnapshot,
    song: &Arc<RwLock<Song>>,
    handle: &mut EngineHandle,
    effects: &[EffectInfo],
    state: &mut MixerViewState,
) -> bool {
    let t = theme();
    let mut delete = false;
    ui.allocate_ui(egui::vec2(STRIP_WIDTH, 0.0), |ui| {
        ui.set_width(STRIP_WIDTH);
        strip_frame(t.colors.bg_panel, t.colors.border).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                // Name — click to rename inline.
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
                    let resp = ui
                        .add(egui::Label::new(
                            RichText::new(format!("⮌ {}", rb.name))
                                .size(t.fonts.size_small)
                                .strong()
                                .color(t.colors.accent_green),
                        ))
                        .on_hover_text("Click to rename");
                    if resp.clicked() {
                        state.editing_return_name = Some((rb.id, rb.name.clone()));
                    }
                }

                // Inserts editor (returns have no other effect editor).
                let fx_btn = ui
                    .button(
                        RichText::new(format!("Inserts ({})", effects.len()))
                            .size(t.fonts.size_small)
                            .color(t.colors.text_secondary),
                    )
                    .on_hover_text("Edit this return bus's effect chain");
                egui::Popup::from_toggle_button_response(&fx_btn).show(|ui| {
                    ui.set_min_width(280.0);
                    draw_return_inserts(ui, rb.id, effects, handle);
                });

                ui.add_space(4.0);

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

                // Mute + delete.
                ui.horizontal(|ui| {
                    ui.add_space(center_pad(20.0 + 22.0));
                    if toggle_chip(ui, "M", rb.mute, t.colors.accent_red)
                        && let Some(bus) = song.write().return_bus_mut(rb.id)
                    {
                        bus.mute = !bus.mute;
                    }
                    if ui
                        .add(egui::Button::new(
                            RichText::new("✖")
                                .size(t.fonts.size_small)
                                .color(t.colors.text_dim),
                        ))
                        .on_hover_text("Delete return bus")
                        .clicked()
                    {
                        delete = true;
                    }
                });

                ui.add_space(4.0);

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
    delete
}

/// Render the return-bus inserts editor inside the popup: list each effect with
/// bypass + remove + parameter sliders, plus an "add effect" picker.
fn draw_return_inserts(
    ui: &mut egui::Ui,
    return_id: ReturnBusId,
    effects: &[EffectInfo],
    handle: &mut EngineHandle,
) {
    let t = theme();
    ui.label(RichText::new("Return inserts").strong());
    ui.separator();

    egui::ScrollArea::vertical()
        .max_height(320.0)
        .show(ui, |ui| {
            if effects.is_empty() {
                ui.label(RichText::new("No effects").color(t.colors.text_dim));
            }
            for fx in effects {
                ui.horizontal(|ui| {
                    let mut enabled = !fx.bypassed;
                    if ui
                        .checkbox(&mut enabled, "")
                        .on_hover_text("Enable / bypass")
                        .changed()
                    {
                        handle.send(EngineCommand::SetReturnEffectEnabled {
                            return_id,
                            module_id: fx.module_id,
                            enabled,
                        });
                    }
                    ui.label(RichText::new(fx.module_type.name()).strong());
                    if ui
                        .button(
                            RichText::new("✖")
                                .size(t.fonts.size_small)
                                .color(t.colors.text_dim),
                        )
                        .on_hover_text("Remove effect")
                        .clicked()
                    {
                        handle.send(EngineCommand::RemoveReturnEffect {
                            return_id,
                            id: fx.module_id,
                        });
                    }
                });

                // Parameter sliders, driven by the effect's descriptor.
                if let Some(descriptor) = cached_descriptor(fx.module_type) {
                    for desc in &descriptor.parameters {
                        let current = fx
                            .params
                            .iter()
                            .find(|p| p.same_kind(&desc.id))
                            .map_or(desc.range.default, Param::as_f32);
                        let mut value = current;
                        let resp = ui.add(
                            egui::Slider::new(&mut value, desc.range.min..=desc.range.max)
                                .text(RichText::new(&desc.name).size(t.fonts.size_small)),
                        );
                        if resp.changed() {
                            handle.send(EngineCommand::SetReturnEffectParameter {
                                return_id,
                                module_id: fx.module_id,
                                param: desc.id.with_f32(value),
                            });
                        }
                    }
                }
                ui.separator();
            }
        });

    // Add-effect picker.
    egui::ComboBox::from_id_salt(("add_return_fx", return_id.0))
        .selected_text(RichText::new("+ Add effect").color(t.colors.accent_green))
        .show_ui(ui, |ui| {
            for &mt in RETURN_FX {
                if ui.selectable_label(false, mt.name()).clicked() {
                    add_return_effect(handle, return_id, mt, effects);
                }
            }
        });
}

/// Create and append a new effect to a return bus's chain. The fresh `ModuleId`
/// instance is one past the highest existing instance of that type on the bus.
fn add_return_effect(
    handle: &mut EngineHandle,
    return_id: ReturnBusId,
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
    handle.send(EngineCommand::AddReturnEffect {
        return_id,
        id: ModuleId::new(module_type, instance),
        effect,
    });
}

/// Draw the master strip. Master volume is engine-owned (atomic + command).
fn draw_master_strip(ui: &mut egui::Ui, handle: &mut EngineHandle) {
    let t = theme();
    let master = handle.state.master_volume.load();
    let (peak_l, peak_r) = handle.peak_meters();
    let level = peak_l.as_f32().max(peak_r.as_f32());
    ui.allocate_ui(egui::vec2(STRIP_WIDTH, 0.0), |ui| {
        ui.set_width(STRIP_WIDTH);
        strip_frame(t.colors.bg_widget, t.colors.border_selected).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("Master")
                        .size(t.fonts.size_small)
                        .strong()
                        .color(t.colors.accent_primary),
                );
                ui.label(
                    RichText::new("Output")
                        .size(t.fonts.size_small)
                        .color(t.colors.text_dim),
                );
                ui.add_space(4.0);

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
