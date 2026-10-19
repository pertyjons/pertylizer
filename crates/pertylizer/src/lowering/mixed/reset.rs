//! Whole-owner edits use the bridge's actual seal, receipt and source handoff.
use super::*;

impl MixedPlayback {
    pub(super) fn prepare_reset(
        &mut self,
        targets: MixedTargetAdmission,
        install: super::super::app::InstallSequence,
    ) -> Result<(), MixedPlaybackError> {
        if self.looping.is_some() {
            return Err(preparation(
                "leave the compiled loop before a whole-owner edit",
            ));
        }
        let at = self.boundary()?;
        let position = if self.playing {
            let elapsed = at
                .as_u64()
                .checked_sub(self.anchor.time().as_u64())
                .ok_or_else(|| preparation("reset precedes actual anchor"))?;
            self.anchor
                .position()
                .checked_add(FrameCount::new(elapsed))
                .map_err(preparation)?
        } else {
            self.resume
        };
        let anchor = StreamAnchor::new(at, position);
        let joined = MixedJoinedStream::open(
            targets,
            StreamAnchor::new(SampleTime::new(0), PlanPosition::new(0)),
        )
        .map_err(|failure| preparation(failure.0))?;
        let prepared = joined
            .prepare_initial()
            .map_err(|failure| preparation(failure.0))?;
        let history = prepared
            .prepare_history(at, position)
            .map_err(preparation)?;
        let suffix = prepared.prepare_suffix(history).map_err(preparation)?;
        let candidate = prepared.stamp_suffix(suffix).map_err(preparation)?;
        let depth = self
            .profile
            .limits()
            .events()
            .queues()
            .performance_ingress_capacity()
            .as_usize()
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| preparation("reset ring overflow"))?;
        let fence = MixedTransferFence {
            stage_cells: MixedTransferPlaces::new(32),
            held_onsets: MixedTransferPlaces::new(16),
            peer_wait: FrameCount::new(64),
        };
        let armed = prepared
            .arm_software_bridge(
                candidate,
                &self.profile,
                &self._connections,
                fence,
                Ring(HeapRb::new(depth)),
                Ring(HeapRb::new(depth)),
            )
            .map_err(|failure| preparation(failure.reason()))?;
        let MixedBridgeArmed {
            control,
            audio,
            control_port,
            audio_port,
        } = armed;
        let owner =
            Owner::new(audio, audio_port).map_err(|_| preparation("reset audio port mismatch"))?;
        let id = self.next_request_id()?;
        self.bridge
            .seal_reset(control, at)
            .map_err(|failure| preparation(failure.reason()))?;
        self.previous = Some(id);
        self.pending = Some(id);
        self.reset_port = Some(control_port);
        self.reset_request = Some(Box::new(Request {
            id,
            operation: Operation::Reset {
                owner: Some(owner),
                retired: Box::new(None),
                refused: None,
                at,
                running: self.playing,
                offered: false,
                install,
                anchor,
                observed_stopped: None,
            },
            complete: false,
            rendered: None,
        }));
        Ok(())
    }
    pub(super) fn publish_ready_reset(&mut self) -> Result<(), MixedPlaybackError> {
        if self.bridge.reset_ready().is_some()
            && let Some(request) = self.reset_request.take()
            && let Err(request) = self.requests.try_push(request)
        {
            self.reset_request = Some(request);
            return Err(preparation("reset operation queue is full"));
        }
        Ok(())
    }
    pub(super) fn collect_reset(
        &mut self,
        owner: Option<Owner>,
        retired: Option<RetiredOwner>,
        refused: Option<synth_engine_v2::stream::MixedBridgeResetRefusal>,
        report: bool,
    ) -> Result<bool, MixedPlaybackError> {
        let next_port = self
            .reset_port
            .take()
            .ok_or_else(|| preparation("reset control port missing"))?;
        if let Some(retired) = retired {
            let receipt = retired.receipt();
            let retired_control = self.bridge.commit_reset(receipt).map_err(preparation)?;
            let old_port = std::mem::replace(&mut self.port, next_port);
            let (audio, audio_port) = retired.into_owner().into_parts();
            let teardown = retired_control
                .join(audio, Some(old_port), Some(audio_port))
                .map_err(|failure| preparation(failure.reason()))?;
            if let Some(fault) = teardown.bridge().fault() {
                return Err(preparation(fault));
            }
            if !teardown.unclassified().is_empty() || !teardown.unmatched().is_empty() {
                return Err(preparation(
                    "reset teardown contains unaccounted source identities",
                ));
            }
            Ok(true)
        } else {
            let owner = owner.ok_or_else(|| preparation("reset fault left an installed secondary owner; terminal cold teardown required"))?;
            let successor = self.bridge.abandon_reset(&owner).map_err(preparation)?;
            let (audio, audio_port) = owner.into_parts();
            let teardown = successor
                .join(audio, Some(next_port), Some(audio_port))
                .map_err(|failure| preparation(failure.reason()))?;
            if let Some(fault) = teardown.fault() {
                return Err(preparation(fault));
            }
            if report {
                self.notice(format!("Mixed reset refused: {refused:?}"));
            }
            Ok(false)
        }
    }
}

impl MixedPlayback {
    /// Quiescent accounting only. No switch acknowledgement or GUI/MCP publication is
    /// written by this cold path, and discarded samples are never output evidence.
    pub(super) fn settle_quiescent(
        &mut self,
        audio: &mut MixedSongAudio,
    ) -> Result<(), MixedPlaybackError> {
        self.collect_operation()?;
        if let Some(request) = self.reset_request.take() {
            self.settle_close_request(*request)?;
        }
        if let Some(request) = audio.pending.take() {
            self.settle_close_request(*request)?;
        }
        while let Some(request) = audio.requests.try_pop() {
            self.settle_close_request(*request)?;
        }
        if let Some(packet) = audio.audio.take_retired_continuation() {
            let _retired = self
                .bridge
                .collect_continuation(packet)
                .map_err(|(_, reason)| preparation(reason))?;
        } else if let Some(packet) = audio.audio.withdraw_continuation() {
            let _cancelled = self
                .bridge
                .cancel_continuation(packet)
                .map_err(|(_, reason)| preparation(reason))?;
        }
        use synth_engine_v2::stream::MixedBridgeResetState;
        if matches!(audio.audio.state(), MixedBridgeResetState::Fading { .. }) {
            // At most Q fade frames remain. Finish ownership accounting, discard all
            // samples, and do not promote any audible installation metadata.
            let mut discarded = [0.0; 128];
            let block = AudioBlockMut::new(
                &mut discarded,
                64,
                synth_engine_v2::quantities::ChannelLayout::Stereo,
            )
            .map_err(preparation)?;
            audio.audio.render(block).map_err(preparation)?;
        }
        if let Some(retired) = audio.audio.take_retired() {
            self.collect_reset(None, Some(retired), None, false)?;
        } else if let Some(refused) = audio.audio.take_refused() {
            let reason = refused.reason();
            self.collect_reset(Some(refused.into_owner()), None, Some(reason), false)?;
        } else if let Some(owner) = audio.audio.withdraw_pending() {
            self.collect_reset(Some(owner), None, None, false)?;
        }
        self.pending = None;
        Ok(())
    }
    pub(super) fn settle_close_request(
        &mut self,
        mut request: Request,
    ) -> Result<(), MixedPlaybackError> {
        match &mut request.operation {
            Operation::Reset {
                owner,
                retired,
                refused,
                ..
            } => {
                if owner.is_some() || retired.is_some() {
                    self.collect_reset(owner.take(), retired.take(), *refused, false)?;
                }
            }
            Operation::Continuation {
                packet, refusal, ..
            } => {
                if let Some(packet) = packet.take() {
                    if packet.outcome().is_some() && refusal.is_none() {
                        let _retired = self
                            .bridge
                            .collect_continuation(packet)
                            .map_err(|(_, reason)| preparation(reason))?;
                    } else {
                        let _cancelled = self
                            .bridge
                            .cancel_continuation(packet)
                            .map_err(|(_, reason)| preparation(reason))?;
                    }
                }
            }
            Operation::Activation { retired, .. } => {
                if let Some(retired) = retired.take() {
                    let _receipt = self
                        .bridge
                        .collect_activation(retired)
                        .map_err(|_| preparation("quiescent activation collection refused"))?;
                }
            }
            Operation::Stop { .. } => {}
        }
        Ok(())
    }
}
