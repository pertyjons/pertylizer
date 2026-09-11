//! The simulated callback: borrowed resources, fixed diagnostics, no ownership transfer.

use super::{CallbackError, ConnectionGeneration, ConnectionState, HostFailure, SimulatedHost};
use crate::render::{AudioBlockMut, Renderer, TimedEvents};
use crate::time::FrameCount;

impl SimulatedHost {
    /// Render only the active Running generation. Stale callbacks silence their own
    /// complete buffer and cannot read or mutate the replacement's renderer or status.
    pub fn callback(
        &mut self,
        generation: ConnectionGeneration,
        mut output: AudioBlockMut<'_>,
    ) -> Result<(), CallbackError> {
        let active = match &mut self.active {
            Some(connection) if connection.status.generation == generation => Some(connection),
            _ => None,
        };
        let (connection, may_play) = match (active, &mut self.candidate) {
            (Some(connection), _) => (connection, true),
            (_, Some(connection)) if connection.status.generation == generation => {
                (connection, false)
            }
            _ => {
                output.silence();
                return Err(CallbackError::StaleGeneration);
            }
        };
        connection.status.last_callback = Some(FrameCount::new(output.frames() as u64));
        if !matches!(
            connection.status.state,
            ConnectionState::Running | ConnectionState::Ready
        ) {
            output.silence();
            return Ok(());
        }
        let Some(prepared) = &mut connection.prepared else {
            output.silence();
            return Err(CallbackError::NotPrepared);
        };
        let invalid_configuration = output.frames() as u64
            > prepared.renderer.plan().maximum_block_size().as_u64()
            || output.layout() != prepared.renderer.plan().channel_layout();
        if (!may_play || connection.status.state == ConnectionState::Ready)
            && !invalid_configuration
        {
            output.silence();
            return Ok(());
        }
        let result = prepared
            .renderer
            .render(output.reborrow(), TimedEvents::new(&[]));
        connection.status.clock = prepared.renderer.clock();
        connection.status.needs_reprepare = prepared.renderer.diagnostics().needs_reprepare();
        if let Err(error) = result {
            output.silence();
            if connection.status.failure.is_none() {
                connection.status.failure = Some(HostFailure::Render(error));
            }
            connection.status.state = ConnectionState::Quiescing;
            return Err(CallbackError::Render(error));
        }
        Ok(())
    }
}
