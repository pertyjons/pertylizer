//! The experimental V2 song-playback toggle (ADR-0077), present only with `v2-lowering`.
//!
//! The switch itself lives in [`crate::lowering::app`]; this module is its wiring into the
//! app shell: the toggle beside the MIDI indicator, the per-frame transport mirror, the
//! project-change notification, MIDI disconnection while V2 is active, and the notices.

use super::SynthApp;
use crate::lowering::app::{PlaybackView, SongSwitch};
use synth_engine::EngineCommand;

/// The parts of the project revision that change what V2 plays. Canvas layout and focus
/// are left out: they change constantly and never reach the lowering. The UI revision stays
/// for GUI-owned edits; a purely cosmetic edit may also rebuild, which is conservative.
/// The command-published instrument snapshot revision catches sound settings such as volume,
/// pan, mute, solo, transpose and key range, including MCP edits that do not advance the graph
/// or GUI revision. The last two are fingerprints of the global
/// settings and the effect order. This tuple detects changes; it does not label one atomic
/// `ProjectFile` snapshot.
type AudibleRevision = (
    synth_core::ContentRevision,
    synth_core::ContentRevision,
    synth_core::ContentRevision,
    synth_core::ContentRevision,
    synth_core::ContentRevision,
    u64,
    u64,
);

fn audible_revision_of(revision: crate::dirty::ProjectRevision) -> AudibleRevision {
    (
        revision.song,
        revision.graph,
        revision.instrument_snapshot,
        revision.samples,
        revision.ui,
        revision.global,
        revision.effect_order,
    )
}

#[cfg(test)]
mod revision_tests {
    use super::*;

    #[test]
    fn command_published_instrument_edits_change_the_playback_revision() {
        let before = crate::dirty::ProjectRevision::default();
        let after = crate::dirty::ProjectRevision {
            instrument_snapshot: synth_core::ContentRevision::new(1),
            ..before
        };
        assert_ne!(audible_revision_of(before), audible_revision_of(after));
    }
}

/// The V2 toggle's state on the app.
#[derive(Default)]
pub(super) struct V2State {
    pub(super) switch: Option<SongSwitch>,
    revision: Option<AudibleRevision>,
    /// The MIDI port disconnected on entering V2 mode, reconnected on leaving it.
    midi_port: Option<String>,
    view: Option<PlaybackView>,
    #[cfg(feature = "mcp")]
    publication_failure: Option<String>,
}

impl V2State {
    pub(super) fn new(switch: SongSwitch) -> Self {
        Self {
            switch: Some(switch),
            revision: None,
            midi_port: None,
            view: None,
            #[cfg(feature = "mcp")]
            publication_failure: None,
        }
    }
}

impl SynthApp {
    fn cache_v2_view(&mut self, switch: &SongSwitch) {
        let view = switch.playback_view();
        #[cfg(feature = "mcp")]
        if let Some(shared) = &self.mcp_shared {
            let result = shared.publish_v2_playback(mcp_playback_state(&view));
            match result {
                Ok(()) => self.v2.publication_failure = None,
                Err(error) if self.v2.publication_failure.as_ref() != Some(&error) => {
                    self.v2.publication_failure = Some(error.clone());
                    self.dialog_state.set_status(error);
                }
                Err(_) => {} // The same observational failure was already surfaced.
            }
        }
        self.v2.view = Some(view);
    }

    fn audible_revision(&self) -> AudibleRevision {
        audible_revision_of(self.current_revision())
    }

    pub(super) fn v2_active(&self) -> bool {
        self.v2.switch.as_ref().is_some_and(SongSwitch::is_active)
    }

    /// Show every notice the switch produced, as a toast and in the log.
    fn show_v2_notices(&mut self, switch: &mut SongSwitch) {
        for notice in switch.take_notices() {
            self.dialog_state.set_status(notice);
        }
    }

    /// Reconnect the MIDI port that entering V2 mode disconnected.
    fn reconnect_midi(&mut self) {
        if let Some(port) = self.v2.midi_port.take()
            && let Err(error) = self.midi_handler.connect_to(&port)
        {
            self.dialog_state
                .set_status(format!("MIDI could not reconnect to {port}: {error}"));
        }
    }

    /// Stop the V1 transport and cut every V1 voice and effect tail at once, so nothing V1
    /// held or rendered silently can become audible when the engines switch.
    fn silence_v1(&mut self) -> bool {
        self.handle.send(EngineCommand::Stop) && self.handle.send(EngineCommand::ResetDsp)
    }

    /// Leave V2 mode: V1 is stopped and reset before the removal, so the callback that
    /// admits the removal lets V1 process the reset before its first audible block. Without
    /// the reset V2 stays. MIDI reconnects in `poll_v2` once the callback has released V2.
    fn leave_v2(&mut self, switch: &mut SongSwitch) {
        if self.silence_v1() {
            switch.disable();
        } else {
            self.dialog_state.set_status(
                "V2 mode kept: V1 could not be reset before leaving it; try again".to_owned(),
            );
        }
    }

    /// Enter or leave V2 mode.
    pub(super) fn toggle_v2(&mut self) {
        let Some(mut switch) = self.v2.switch.take() else {
            return;
        };
        switch.track_identity_source(std::sync::Arc::clone(self.session.state()));
        if switch.is_active() {
            self.leave_v2(&mut switch);
        } else if !switch.is_released() {
            // The previous V2 session is still leaving the callback; MIDI stays off and its
            // saved port stays saved until then.
            self.dialog_state
                .set_status("V2 playback is still switching off; try again".to_owned());
        } else {
            // MIDI goes first, so no new note can start; then V1 is stopped and reset; only
            // then is V2 entered. If a later step fails, MIDI reconnects and V1 stays.
            self.v2.midi_port = self.midi_handler.disconnect();
            if self.silence_v1() {
                // Read before building: an edit during the multi-owner snapshot
                // must remain visible to the next poll instead of being labelled
                // as already included in the installed plan.
                let revision = self.audible_revision();
                if switch.enable_from(|| self.create_project_from_app()) {
                    self.v2.revision = Some(revision);
                } else {
                    self.reconnect_midi();
                }
            } else {
                self.reconnect_midi();
                self.dialog_state.set_status(
                    "V2 playback not entered: V1 could not be stopped first".to_owned(),
                );
            }
        }
        self.show_v2_notices(&mut switch);
        self.cache_v2_view(&switch);
        self.v2.switch = Some(switch);
    }

    #[cfg(feature = "v2-mixed-app")]
    fn toggle_mixed(&mut self) {
        let Some(mut switch) = self.v2.switch.take() else {
            return;
        };
        switch.track_identity_source(std::sync::Arc::clone(self.session.state()));
        if switch.mixed_active() {
            self.leave_v2(&mut switch);
        } else if switch.is_released() {
            self.v2.midi_port = self.midi_handler.disconnect();
            if self.silence_v1() {
                if let Some(instrument) = self.active_instrument_id {
                    let revision = self.audible_revision();
                    if switch.enable_mixed_from(|| self.create_project_from_app(), instrument) {
                        self.v2.revision = Some(revision);
                    } else {
                        self.reconnect_midi();
                    }
                } else {
                    self.dialog_state.set_status(
                        "Select a live instrument before running mixed playback".to_owned(),
                    );
                    self.reconnect_midi();
                }
            } else {
                self.reconnect_midi();
            }
        } else {
            self.dialog_state
                .set_status("Leave the current V2 route before entering mixed playback".to_owned());
        }
        self.show_v2_notices(&mut switch);
        self.cache_v2_view(&switch);
        self.v2.switch = Some(switch);
    }

    #[cfg(feature = "v2-mixed-app")]
    pub(super) fn mixed_note(
        &mut self,
        source: crate::lowering::app::SoftwareSource,
        note: synth_core::MidiNote,
        down: bool,
    ) -> bool {
        let Some(switch) = &mut self.v2.switch else {
            return false;
        };
        if !switch.mixed_active() {
            return false;
        }
        switch.mixed_note(source, note, down);
        true
    }

    /// Per frame: service the V2 session, report project changes, and reconnect MIDI once
    /// the callback has released V2.
    pub(super) fn poll_v2(&mut self) {
        let Some(mut switch) = self.v2.switch.take() else {
            return;
        };
        switch.track_identity_source(std::sync::Arc::clone(self.session.state()));
        switch.poll(|| self.create_project_from_app());
        if switch.needs_disable() {
            // The edited project was refused; leave through the path that resets V1 first.
            self.leave_v2(&mut switch);
        } else if switch.is_active() {
            let revision = self.audible_revision();
            if self.v2.revision != Some(revision) {
                self.v2.revision = Some(revision);
                switch.project_changed(|| self.create_project_from_app());
            }
        } else if switch.is_released() {
            self.reconnect_midi();
        }
        self.show_v2_notices(&mut switch);
        self.cache_v2_view(&switch);
        self.v2.switch = Some(switch);
    }

    /// Whether the MIDI port selector must stay closed: MIDI input is off in V2 mode and
    /// until the callback has released V2.
    pub(super) fn midi_locked_by_v2(&self) -> bool {
        self.v2
            .switch
            .as_ref()
            .is_some_and(|switch| !switch.is_released())
    }

    /// The top-bar toggle beside the MIDI indicator, and V2's own transport while active.
    pub(super) fn render_v2_toggle(&mut self, ui: &mut egui::Ui) {
        use egui_remixicon::icons as ri;
        let active = self.v2_active();
        #[cfg(feature = "v2-mixed-app")]
        {
            let mixed = self
                .v2
                .switch
                .as_ref()
                .is_some_and(SongSwitch::mixed_active);
            if ui.selectable_label(mixed, "Run mixed").on_hover_text(
                "Experimental score and two software keyboards; unqualified capacity and Arrival timing; MIDI, recording and mixed tap observers unavailable"
            ).clicked() { self.toggle_mixed(); }
            if mixed
                && let Some(instrument) = self
                    .v2
                    .switch
                    .as_ref()
                    .and_then(SongSwitch::mixed_instrument)
            {
                ui.label(format!("Live: {instrument:?}"));
            }
        }
        #[cfg_attr(not(feature = "v2-mixed-app"), allow(unused_mut))]
        let mut hover = if active {
            "Experimental V2 song playback is active: song only, MIDI off, no seek, loop or \
             pattern preview; the V1 transport does not affect it. Click to return to V1."
        } else {
            "Play the song through the experimental V2 engine (ADR-0077). Only projects V2 \
             can lower will play; entering stops V1."
        };
        #[cfg(feature = "v2-mixed-app")]
        if self
            .v2
            .switch
            .as_ref()
            .is_some_and(SongSwitch::mixed_active)
        {
            hover = "Experimental mixed playback: score and software keys, MIDI and recording off; compiled seek and loop available. Click to return to V1.";
        }
        let response = ui.selectable_label(active, "V2").on_hover_text(hover);
        if response.clicked() {
            self.toggle_v2();
        }
        if !active {
            return;
        }
        #[cfg(feature = "v2-mixed-app")]
        if self
            .v2
            .switch
            .as_ref()
            .is_some_and(SongSwitch::mixed_active)
        {
            let looping = self
                .v2
                .switch
                .as_ref()
                .is_some_and(SongSwitch::mixed_looping);
            if ui
                .button(if looping {
                    "Leave loop and rewind"
                } else {
                    "Loop song from start"
                })
                .clicked()
                && let Some(switch) = &mut self.v2.switch
            {
                switch.mixed_loop(!looping);
            }
            if ui.button("Seek start").clicked()
                && let Some(switch) = &mut self.v2.switch
            {
                switch.mixed_seek_start();
            }
        }
        let playing = self.v2.switch.as_ref().is_some_and(SongSwitch::is_playing);
        let (icon, hover) = if playing {
            (ri::PAUSE_FILL, "Pause V2 playback")
        } else {
            (ri::PLAY_FILL, "Play V2 from the start or where it paused")
        };
        if ui.button(icon).on_hover_text(hover).clicked()
            && let Some(mut switch) = self.v2.switch.take()
        {
            if playing {
                switch.pause();
            } else {
                switch.play(|| self.create_project_from_app());
            }
            self.show_v2_notices(&mut switch);
            self.cache_v2_view(&switch);
            self.v2.switch = Some(switch);
        }
        if ui
            .button(ri::STOP_FILL)
            .on_hover_text("Stop V2 and return to the song start")
            .clicked()
            && let Some(mut switch) = self.v2.switch.take()
        {
            switch.stop(|| self.create_project_from_app());
            self.show_v2_notices(&mut switch);
            self.cache_v2_view(&switch);
            self.v2.switch = Some(switch);
        }
    }

    /// The status-bar badge listing what V2 omits from the current project.
    pub(super) fn render_v2_badge(&self, ui: &mut egui::Ui) {
        let Some(view) = &self.v2.view else {
            return;
        };
        if !view.active {
            if view.releasing {
                ui.label("V2 switching off");
            }
            return;
        }
        let omissions = &view.omissions;
        let rendered = view.rendered;
        let text = match (rendered, omissions.len()) {
            (Some(plan), 0) => format!("V2 {}", plan.plan_id()),
            (Some(plan), count) => format!("V2 {}: {count} omitted", plan.plan_id()),
            (None, 0) => "V2 plan unconfirmed".to_owned(),
            (None, count) => format!("V2 plan unconfirmed: {count} omitted"),
        };
        let tooltip = match rendered {
            None => {
                let pending =
                    "The latest V2 plan has not completed a callback, or V2 output is silent.";
                if omissions.is_empty() {
                    pending.to_owned()
                } else {
                    format!(
                        "{pending}\nV2 lowering omissions:\n{}",
                        omissions.join("\n")
                    )
                }
            }
            Some(plan) => {
                let source = format!(
                    "The last completed V2 callback rendered this plan. Captured lowering-input equality token (this service instance only): {}",
                    plan.project_digest()
                );
                if omissions.is_empty() {
                    format!("{source}\nLowering reported no omissions.")
                } else {
                    format!("{source}\nV2 lowering omissions:\n{}", omissions.join("\n"))
                }
            }
        };
        let scope = view.scope.map_or_else(
            || "unavailable".to_owned(),
            |scope| scope.value().to_string(),
        );
        let install = rendered.map_or_else(
            || "unconfirmed".to_owned(),
            |plan| plan.install().as_u64().to_string(),
        );
        let tooltip = format!(
            "{tooltip}\nSession {scope}, installation {install}. Requested playing: {}; edit pending: {}; refused: {}. This is a session observation, not a canonical project revision or device heartbeat.",
            view.requested_playing, view.edit_pending, view.refused
        );
        ui.label(text).on_hover_text(tooltip);
    }
}

#[cfg(feature = "mcp")]
fn mcp_playback_state(
    view: &PlaybackView,
) -> Result<synth_mcp::v2_playback::PlaybackState, crate::mcp_shared::V2PublicationError> {
    use crate::mcp_shared::V2PublicationError;
    use synth_mcp::v2_playback::*;
    if !view.active {
        return Ok(PlaybackState::Inactive {
            releasing: view.releasing,
        });
    }
    let scope = SwitchScope::new(
        view.scope
            .ok_or(V2PublicationError::IdentityExhausted)?
            .value(),
    );
    if view.refused {
        return Ok(PlaybackState::Refused { scope });
    }
    let audible = view
        .rendered
        .map(|rendered| -> Result<AudiblePlan, V2PublicationError> {
            let install = std::num::NonZeroU64::new(rendered.install().as_u64())
                .ok_or(V2PublicationError::InvalidObservation)?;
            let plan = std::num::NonZeroU64::new(rendered.plan_id().as_u64())
                .ok_or(V2PublicationError::InvalidObservation)?;
            let digest = LoweringDigest::try_from(rendered.project_digest().to_string())
                .map_err(|_| V2PublicationError::InvalidObservation)?;
            Ok(AudiblePlan {
                install: RenderedInstall::new(install),
                plan: RenderedPlan::new(plan),
                projection: LoweringProjection::V1,
                digest,
                omissions: view.omissions.clone(),
            })
        })
        .transpose()?;
    Ok(PlaybackState::Active {
        scope,
        requested_playing: view.requested_playing,
        edit_pending: view.edit_pending,
        audible,
    })
}

#[cfg(all(test, feature = "mcp"))]
mod publication_tests {
    use super::*;
    use synth_core::audio::{
        AudioCallbackContext, AudioProcessor, BufferSize, ChannelCount, DeviceSampleRate,
        StreamInfo,
    };
    use synth_mcp::v2_playback::*;
    struct Silent;
    impl AudioProcessor for Silent {
        fn process(&mut self, output: &mut [f32], _: &AudioCallbackContext) {
            output.fill(0.0);
        }
    }
    #[cfg(feature = "v2-mixed-app")]
    #[test]
    fn mixed_gui_and_mcp_share_actual_callback_confirmations_across_a_plan_edit() {
        let (mut control, mut audio) = crate::lowering::app::wrap(Silent);
        control.set_stream(&StreamInfo {
            sample_rate: DeviceSampleRate::DVD_QUALITY,
            buffer_size: BufferSize::MEDIUM,
            channels: ChannelCount::Stereo,
            output_latency: std::time::Duration::ZERO,
            input_latency: None,
        });
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpus/v2-reference/projects/subtractive-voice.ptz");
        let mut project = crate::project::ProjectFile::load(path).unwrap();
        let shared = crate::mcp_shared::McpSharedState::new();
        shared.attach_gui_once(|| Box::new(|| {}));
        assert!(control.enable_mixed_from(|| project.clone(), project.instruments[0].id));
        let queued = control.playback_view();
        shared
            .publish_v2_playback(mcp_playback_state(&queued))
            .unwrap();
        assert!(queued.rendered.is_none());
        let mut block = [0.0; 512];
        let context = AudioCallbackContext {
            sample_rate: DeviceSampleRate::DVD_QUALITY,
            frames: 256,
            channels: 2,
            stream_time: 0.0,
            sample_position: 0,
            output_latency: synth_core::Seconds::ZERO,
        };
        let measured = allocation_counter::measure(|| audio.process(&mut block, &context));
        assert_eq!((measured.count_total, measured.count_current), (0, 0));
        let old = control.playback_view().rendered.unwrap();
        project.global.master_volume = synth_core::Gain::new(0.37);
        control.project_changed(|| project.clone());
        let pending = control.playback_view();
        assert_eq!(pending.rendered, Some(old));
        assert!(pending.edit_pending);
        for _ in 0..40 {
            let measured = allocation_counter::measure(|| audio.process(&mut block, &context));
            assert_eq!((measured.count_total, measured.count_current), (0, 0));
            control.poll(|| project.clone());
        }
        let confirmed = control.playback_view();
        let new = confirmed.rendered.unwrap();
        assert_ne!(new.plan_id(), old.plan_id());
        assert!(!confirmed.edit_pending);
        shared
            .publish_v2_playback(mcp_playback_state(&confirmed))
            .unwrap();
        match shared.v2_playback_status().unwrap().observation {
            PlaybackObservation::Published {
                state:
                    PlaybackState::Active {
                        audible: Some(audible),
                        ..
                    },
                ..
            } => {
                assert_eq!(
                    String::from(audible.plan),
                    format!("plan-{}", new.plan_id().as_u64())
                );
                assert_eq!(
                    String::from(audible.digest),
                    new.project_digest().to_string()
                );
                assert_eq!(audible.projection, LoweringProjection::V1);
            }
            other => panic!("mixed plan was not published: {other:?}"),
        }
        control.disable();
        let removed = control.playback_view();
        assert!(removed.rendered.is_none());
        shared
            .publish_v2_playback(mcp_playback_state(&removed))
            .unwrap();
        audio.process(&mut block, &context);
        control.poll(|| project.clone());
        assert!(control.is_released());
        assert!(control.take_notices().is_empty());
    }

    #[test]
    fn private_publisher_lifecycle_controls_run_in_the_required_v2_flow_gate() {
        crate::mcp_shared::assert_v2_publisher_contracts();
    }

    #[test]
    fn the_gui_view_publishes_only_callback_confirmed_plans_and_never_queued_or_muted_plans() {
        let (mut control, mut audio) = crate::lowering::app::wrap(Silent);
        control.set_stream(&StreamInfo {
            sample_rate: DeviceSampleRate::DVD_QUALITY,
            buffer_size: BufferSize::MEDIUM,
            channels: ChannelCount::Stereo,
            output_latency: std::time::Duration::ZERO,
            input_latency: None,
        });
        let project = crate::project::ProjectFile::new(
            Vec::new(),
            0,
            None,
            synth_sequencer::Song::new("publication"),
            crate::project::GlobalProjectState::default(),
        );
        let shared = crate::mcp_shared::McpSharedState::new();
        shared.attach_gui_once(|| Box::new(|| {}));
        assert!(control.enable_from(|| project.clone()));
        let queued = control.playback_view();
        shared
            .publish_v2_playback(mcp_playback_state(&queued))
            .unwrap();
        assert!(matches!(
            shared.v2_playback_status().unwrap().observation,
            PlaybackObservation::Published {
                state: PlaybackState::Active { audible: None, .. },
                ..
            }
        ));
        let mut block = [0.0; 512];
        let context = AudioCallbackContext {
            sample_rate: DeviceSampleRate::DVD_QUALITY,
            frames: 256,
            channels: 2,
            stream_time: 0.0,
            sample_position: 0,
            output_latency: synth_core::Seconds::ZERO,
        };
        let measured = allocation_counter::measure(|| audio.process(&mut block, &context));
        assert_eq!((measured.count_total, measured.count_current), (0, 0));
        assert!(
            queued.rendered.is_none(),
            "a cached view does not change with the callback"
        );
        let confirmed = control.playback_view();
        let rendered = confirmed.rendered.unwrap();
        shared
            .publish_v2_playback(mcp_playback_state(&confirmed))
            .unwrap();
        match shared.v2_playback_status().unwrap().observation {
            PlaybackObservation::Published {
                state:
                    PlaybackState::Active {
                        audible: Some(audible),
                        ..
                    },
                ..
            } => {
                assert_eq!(
                    String::from(audible.plan),
                    format!("plan-{}", rendered.plan_id().as_u64())
                );
                assert_eq!(
                    String::from(audible.install),
                    format!("install-{}", rendered.install().as_u64())
                );
                assert_eq!(
                    String::from(audible.digest),
                    rendered.project_digest().to_string()
                );
                assert_eq!(audible.omissions, confirmed.omissions);
            }
            other => panic!("{other:?}"),
        }
        control.stop(|| project.clone());
        let stopped = control.playback_view();
        assert!(stopped.active);
        assert!(stopped.rendered.is_none());
        shared
            .publish_v2_playback(mcp_playback_state(&stopped))
            .unwrap();
        assert!(matches!(
            shared.v2_playback_status().unwrap().observation,
            PlaybackObservation::Published {
                state: PlaybackState::Active { audible: None, .. },
                ..
            }
        ));
        control.disable();
        let removed = control.playback_view();
        assert!(removed.rendered.is_none());
        shared
            .publish_v2_playback(mcp_playback_state(&removed))
            .unwrap();
        assert!(matches!(
            shared.v2_playback_status().unwrap().observation,
            PlaybackObservation::Published {
                state: PlaybackState::Inactive { releasing: true },
                ..
            }
        ));
        let mut refused = confirmed.clone();
        refused.refused = true;
        assert!(matches!(
            mcp_playback_state(&refused).unwrap(),
            PlaybackState::Refused { .. }
        ));
        let mut exhausted = confirmed;
        exhausted.scope = None;
        assert!(
            shared
                .publish_v2_playback(mcp_playback_state(&exhausted))
                .is_err()
        );
        assert_eq!(
            shared.v2_playback_status().unwrap(),
            V2PlaybackStatus::unavailable(UnavailableReason::IdentityExhausted)
        );
    }
}
