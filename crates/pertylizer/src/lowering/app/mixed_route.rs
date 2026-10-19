//! Feature-gated entry points; the ordinary V2 route keeps its compiled-only policy.
use super::*;
use crate::lowering::mixed::LiveBinding;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum SoftwareSource {
    ComputerKeyboard,
    OnScreenKeyboard,
}
impl SoftwareSource {
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::ComputerKeyboard => 0,
            Self::OnScreenKeyboard => 1,
        }
    }
}

impl SongSwitch {
    pub const fn last_mixed_retirement(&self) -> Option<MixedRetirement> {
        self.last_mixed_retirement
    }
    #[cfg(test)]
    pub(crate) fn mixed_reset_pending_for_test(&self) -> bool {
        self.mixed
            .as_ref()
            .is_some_and(|mixed| mixed.reset_pending_for_test())
    }
    #[cfg(test)]
    pub(crate) fn mixed_stopped_position_for_test(
        &self,
    ) -> Option<synth_engine_v2::time::PlanPosition> {
        self.mixed
            .as_ref()
            .and_then(|mixed| mixed.stopped_position_for_test())
    }
    #[cfg(test)]
    pub(crate) fn mixed_anchor_time_for_test(&self) -> Option<synth_engine_v2::time::SampleTime> {
        self.mixed
            .as_ref()
            .map(|mixed| mixed.anchor_time_for_test())
    }
    #[cfg(test)]
    pub(crate) fn mixed_anchor_position_for_test(
        &self,
    ) -> Option<synth_engine_v2::time::PlanPosition> {
        self.mixed
            .as_ref()
            .map(|mixed| mixed.anchor_position_for_test())
    }
    pub fn mixed_active(&self) -> bool {
        self.mixed.is_some()
    }
    pub fn enable_mixed(
        &mut self,
        project: &crate::project::ProjectFile,
        instrument: synth_core::InstrumentId,
    ) -> bool {
        if let Err(error) = self.prepare_mixed_install(project, instrument) {
            self.notices
                .push(format!("Mixed playback refused: {error}"));
            return false;
        }
        true
    }
    pub fn enable_mixed_from(
        &mut self,
        project: impl FnOnce() -> crate::project::ProjectFile,
        instrument: synth_core::InstrumentId,
    ) -> bool {
        let snapshot = self.snapshot(project);
        if self.enable_mixed(&snapshot.project, instrument) {
            self.identity_lineage = snapshot.lineage;
            true
        } else {
            false
        }
    }
    fn prepare_mixed_install(
        &mut self,
        project: &crate::project::ProjectFile,
        instrument: synth_core::InstrumentId,
    ) -> Result<(), String> {
        if !self.is_released() || self.retiring_mixed.len() == SLOTS {
            return Err("wait for the previous audio owner to retire".to_owned());
        }
        let profile = self.profile()?;
        if ![44_100.0, 48_000.0, 96_000.0, 192_000.0]
            .contains(&profile.capabilities().sample_rate().as_f32())
        {
            return Err("mixed software playback admits only 44.1, 48, 96 or 192 kHz".to_owned());
        }
        let digest = LoweredProjectDigest::of(project).map_err(|error| error.to_string())?;
        let (mut control, mut audio) =
            super::super::mixed::prepare(project, profile, LiveBinding::software(instrument))
                .map_err(|error| error.to_string())?;
        let plan = audio.plan_id();
        let sequence = self
            .sent
            .next()
            .ok_or_else(|| "installation identities exhausted".to_owned())?;
        audio.set_install_sequence(sequence);
        control.set_owner_install(sequence);
        self.commands
            .try_push(SwitchCommand::InstallMixed(sequence, Box::new(audio)))
            .map_err(|_| "switch command ring is full".to_owned())?;
        self.omissions = control.omissions().to_vec();
        self.latest_plan = Some(RenderedPlan {
            install: sequence,
            plan,
            project: digest,
        });
        self.identity_lineage = self
            .identity_source
            .as_ref()
            .and_then(|source| source.shared_graph.identity_lineage());
        self.muted_until.store(sequence.as_u64(), Ordering::Release);
        self.mixed = Some(control);
        self.sent = sequence;
        self.occupied = true;
        Ok(())
    }
    pub fn mixed_instrument(&self) -> Option<synth_core::InstrumentId> {
        self.mixed
            .as_ref()
            .map(|mixed| mixed.binding().instrument())
    }
    pub(super) fn mixed_project_changed(&mut self, snapshot: &EditSnapshot) {
        if self.pending_running.is_some()
            || self.mixed.as_ref().is_some_and(|mixed| mixed.editing())
        {
            self.rebuild_pending = true;
            return;
        }
        let Some(install) = self.sent.next() else {
            self.notices
                .push("Mixed installation identities exhausted; prepare a new switch".to_owned());
            self.disable_pending = true;
            return;
        };
        let digest = match LoweredProjectDigest::of(&snapshot.project) {
            Ok(digest) => digest,
            Err(error) => {
                self.notices.push(error.to_string());
                self.disable_pending = true;
                return;
            }
        };
        let source = self.identity_source.clone();
        let old_lineage = self.identity_lineage;
        let lineage = snapshot.lineage;
        let Some(mixed) = &mut self.mixed else {
            return;
        };
        match mixed.replace(&snapshot.project, install, move || {
            source.as_ref().is_none_or(|source| {
                lineage.is_some()
                    && old_lineage == lineage
                    && source.shared_graph.identity_lineage() == lineage
            })
        }) {
            Ok(plan) => {
                self.pending_running = Some(RunningPublication {
                    install,
                    previous: self.latest_plan,
                    omissions: mixed.candidate_omissions().to_vec(),
                    superseded: false,
                    lineage: snapshot.lineage,
                });
                self.latest_plan = Some(RenderedPlan {
                    install,
                    plan,
                    project: digest,
                });
                self.sent = install;
                self.rebuild_pending = false;
                self.running_declined = false;
            }
            Err(error) => {
                self.notices
                    .push(format!("Mixed edit remains pending: {error}"));
                self.rebuild_pending = true;
                self.running_declined = true;
            }
        }
    }

    pub(super) fn reconcile_mixed_edit(&mut self) {
        let completion = self
            .mixed
            .as_mut()
            .and_then(|mixed| mixed.take_edit_completion());
        if let Some((install, applied)) = completion {
            if let Some(pending) = self.pending_running.take() {
                if pending.install != install {
                    self.disable_pending = true;
                    return;
                }
                if applied {
                    self.omissions = pending.omissions;
                    self.identity_lineage = pending.lineage;
                } else {
                    self.latest_plan = pending.previous;
                    self.rebuild_pending = true;
                    self.running_declined = true;
                }
            } else {
                self.disable_pending = true;
            }
        }
    }

    pub fn mixed_note(&mut self, source: SoftwareSource, note: synth_core::MidiNote, down: bool) {
        let bytes = [
            if down { 0x90 } else { 0x80 },
            note.as_u8(),
            if down { 102 } else { 0 },
        ];
        match synth_engine_v2::recording::notes::Midi1Input::from_bytes(bytes) {
            Ok(input) => self.mixed_input(source, input),
            Err(error) => self.notices.push(format!("Mixed software key: {error}")),
        }
    }
    pub fn mixed_seek_start(&mut self) {
        self.mixed_seek(synth_engine_v2::time::PlanPosition::ZERO);
    }
    pub fn mixed_input(
        &mut self,
        source: SoftwareSource,
        input: synth_engine_v2::recording::notes::Midi1Input,
    ) {
        let Some(mixed) = &mut self.mixed else {
            return;
        };
        if let Err(error) = mixed.stage(source.index(), input) {
            self.notices.push(error.to_string());
        }
    }
    pub fn mixed_looping(&self) -> bool {
        self.mixed.as_ref().is_some_and(|mixed| mixed.looping())
    }
    pub fn mixed_loop(&mut self, enabled: bool) {
        if !enabled {
            self.running_declined = false;
        }
        let Some(mixed) = &mut self.mixed else {
            return;
        };
        if let Err(error) = mixed.loop_song(enabled) {
            self.notices.push(error.to_string());
        }
    }
    pub fn release_mixed_sources(&mut self) {
        let Some(mixed) = &mut self.mixed else {
            return;
        };
        for source in [
            SoftwareSource::ComputerKeyboard,
            SoftwareSource::OnScreenKeyboard,
        ] {
            if let Err(error) = mixed.release_source(source.index()) {
                self.notices.push(error.to_string());
            }
        }
    }
    pub fn mixed_seek(&mut self, position: synth_engine_v2::time::PlanPosition) {
        self.running_declined = false;
        let Some(mixed) = &mut self.mixed else {
            return;
        };
        if let Err(error) = mixed.seek(position) {
            self.notices.push(error.to_string());
        }
    }
    pub fn mixed_pause(&mut self, rewind: bool) {
        self.running_declined = false;
        let Some(mixed) = &mut self.mixed else {
            return;
        };
        if let Err(error) = mixed.pause(rewind) {
            self.notices.push(error.to_string());
        }
    }
    pub fn mixed_play(&mut self) {
        let Some(mixed) = &mut self.mixed else {
            return;
        };
        if let Err(error) = mixed.play() {
            self.notices.push(error.to_string());
        }
    }
}
