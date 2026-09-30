//! The experimental V2 song-playback toggle (ADR-0077), present only with `v2-lowering`.
//!
//! The switch itself lives in [`crate::lowering::app`]; this module is its wiring into the
//! app shell: the toggle beside the MIDI indicator, the per-frame transport mirror, the
//! project-change notification, MIDI disconnection while V2 is active, and the notices.

use super::SynthApp;
use crate::lowering::app::SongSwitch;
use synth_engine::EngineCommand;

/// The parts of the project revision that change what V2 plays. Canvas layout and focus
/// are left out: they change constantly and never reach the lowering. The UI revision stays:
/// it carries instrument volume, pan, mute, solo, transpose and key range, which V2 plays, so
/// a purely cosmetic instrument edit also rebuilds, which is conservative.
/// The last two are fingerprints of the global settings and the effect order.
type AudibleRevision = (
    synth_core::ContentRevision,
    synth_core::ContentRevision,
    synth_core::ContentRevision,
    synth_core::ContentRevision,
    u64,
    u64,
);

/// The V2 toggle's state on the app.
#[derive(Default)]
pub(super) struct V2State {
    switch: Option<SongSwitch>,
    revision: Option<AudibleRevision>,
    /// The MIDI port disconnected on entering V2 mode, reconnected on leaving it.
    midi_port: Option<String>,
}

impl V2State {
    pub(super) fn new(switch: SongSwitch) -> Self {
        Self {
            switch: Some(switch),
            revision: None,
            midi_port: None,
        }
    }
}

impl SynthApp {
    fn audible_revision(&self) -> AudibleRevision {
        let revision = self.current_revision();
        (
            revision.song,
            revision.graph,
            revision.samples,
            revision.ui,
            revision.global,
            revision.effect_order,
        )
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
                let project = self.create_project_from_app();
                if switch.enable(&project) {
                    self.v2.revision = Some(self.audible_revision());
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
        self.v2.switch = Some(switch);
    }

    /// Per frame: service the V2 session, report project changes, and reconnect MIDI once
    /// the callback has released V2.
    pub(super) fn poll_v2(&mut self) {
        let Some(mut switch) = self.v2.switch.take() else {
            return;
        };
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
        let response = ui.selectable_label(active, "V2").on_hover_text(if active {
            "Experimental V2 song playback is active: song only, MIDI off, no seek, loop or \
             pattern preview; the V1 transport does not affect it. Click to return to V1."
        } else {
            "Play the song through the experimental V2 engine (ADR-0077). Only projects V2 \
             can lower will play; entering stops V1."
        });
        if response.clicked() {
            self.toggle_v2();
        }
        if !active {
            return;
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
            self.v2.switch = Some(switch);
        }
    }

    /// The status-bar badge listing what V2 omits from the current project.
    pub(super) fn render_v2_badge(&self, ui: &mut egui::Ui) {
        let Some(switch) = self.v2.switch.as_ref().filter(|s| s.is_active()) else {
            return;
        };
        let omissions = switch.omissions();
        let text = if omissions.is_empty() {
            "V2".to_owned()
        } else {
            format!("V2: {} omitted", omissions.len())
        };
        let tooltip = if omissions.is_empty() {
            "V2 song playback is active and represents the whole project.".to_owned()
        } else {
            omissions.join("\n")
        };
        ui.label(text).on_hover_text(tooltip);
    }
}
