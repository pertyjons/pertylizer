//! Transactional command batching for atomic operations.
//!
//! This module provides support for grouping multiple engine commands
//! into atomic batches that succeed or fail together, useful for
//! patch loading and complex multi-module operations.

use std::sync::atomic::{AtomicU64, Ordering};

use super::commands::{EngineCommand, ModuleId, PortId};
use super::instrument::InstrumentId;
use super::typed_params::{ModuleType, Param};

/// Global transaction ID counter.
static NEXT_TRANSACTION_ID: AtomicU64 = AtomicU64::new(1);

/// Unique identifier for a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransactionId(pub u64);

impl TransactionId {
    /// Generate a new unique transaction ID.
    pub fn new() -> Self {
        Self(NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for TransactionId {
    fn default() -> Self {
        Self::new()
    }
}

/// A command within a transaction, with metadata.
#[derive(Debug)]
pub struct TransactionalCommand {
    /// The actual command.
    pub command: EngineCommand,
    /// Whether this command can be rolled back.
    pub reversible: bool,
    /// Optional reverse command (for rollback).
    pub reverse: Option<Box<EngineCommand>>,
    /// Ordering priority within the batch.
    pub priority: u32,
}

impl TransactionalCommand {
    /// Create a new transactional command.
    pub fn new(command: EngineCommand) -> Self {
        Self {
            command,
            reversible: false,
            reverse: None,
            priority: 100, // Default priority
        }
    }

    /// Create a reversible command with its reverse.
    pub fn reversible(command: EngineCommand, reverse: EngineCommand) -> Self {
        Self {
            command,
            reversible: true,
            reverse: Some(Box::new(reverse)),
            priority: 100,
        }
    }

    /// Set the priority (lower = execute first).
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }
}

/// A batch of commands to execute atomically.
#[derive(Debug)]
pub struct CommandBatch {
    /// Transaction ID.
    pub id: TransactionId,
    /// Commands in this batch.
    commands: Vec<TransactionalCommand>,
    /// Description of this batch (for logging/debugging).
    pub description: String,
    /// Whether to validate topology after batch.
    pub validate_topology: bool,
    /// Whether to send confirmation event after completion.
    pub send_confirmation: bool,
}

impl CommandBatch {
    /// Create a new empty batch.
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            id: TransactionId::new(),
            commands: Vec::new(),
            description: description.into(),
            validate_topology: true,
            send_confirmation: true,
        }
    }

    /// Create a batch for loading a patch.
    pub fn for_patch_load(patch_name: impl Into<String>) -> Self {
        Self::new(format!("Load patch: {}", patch_name.into()))
    }

    /// Add a command to the batch.
    pub fn add(&mut self, command: EngineCommand) -> &mut Self {
        self.commands.push(TransactionalCommand::new(command));
        self
    }

    /// Add a reversible command to the batch.
    pub fn add_reversible(&mut self, command: EngineCommand, reverse: EngineCommand) -> &mut Self {
        self.commands
            .push(TransactionalCommand::reversible(command, reverse));
        self
    }

    /// Add a command with priority.
    pub fn add_with_priority(&mut self, command: EngineCommand, priority: u32) -> &mut Self {
        self.commands
            .push(TransactionalCommand::new(command).with_priority(priority));
        self
    }

    /// Add a module to the batch (to global graph by default).
    pub fn add_module(
        &mut self,
        id: ModuleId,
        module: Box<dyn crate::modules::PolyModule>,
    ) -> &mut Self {
        self.add_module_to(None, id, module)
    }

    /// Add a module to a specific instrument's voice graph or global graph.
    pub fn add_module_to(
        &mut self,
        instrument_id: Option<InstrumentId>,
        id: ModuleId,
        module: Box<dyn crate::modules::PolyModule>,
    ) -> &mut Self {
        self.add_with_priority(
            EngineCommand::AddModuleInstance {
                instrument_id,
                id,
                module,
            },
            10, // Modules added first
        )
    }

    /// Add a connection to the batch (to global graph by default).
    pub fn add_connection(&mut self, from: PortId, to: PortId) -> &mut Self {
        self.add_connection_to(None, from, to)
    }

    /// Add a connection to a specific instrument's voice graph or global graph.
    pub fn add_connection_to(
        &mut self,
        instrument_id: Option<InstrumentId>,
        from: PortId,
        to: PortId,
    ) -> &mut Self {
        self.add_with_priority(
            EngineCommand::Connect {
                instrument_id,
                from: from.clone(),
                to: to.clone(),
            },
            50, // Connections after modules
        );
        self
    }

    /// Add a parameter set to the batch (to global graph by default).
    /// The Param contains both the parameter type and its value.
    pub fn set_parameter(&mut self, module_id: ModuleId, param: Param) -> &mut Self {
        self.set_parameter_on(None, module_id, param)
    }

    /// Set a parameter on a specific instrument's voice graph or global graph.
    pub fn set_parameter_on(
        &mut self,
        instrument_id: Option<InstrumentId>,
        module_id: ModuleId,
        param: Param,
    ) -> &mut Self {
        self.add_with_priority(
            EngineCommand::SetModuleParameter {
                instrument_id,
                module_id,
                param,
            },
            90, // Parameters after connections
        )
    }

    /// Get all commands, sorted by priority.
    pub fn commands(&self) -> Vec<&TransactionalCommand> {
        let mut sorted: Vec<_> = self.commands.iter().collect();
        sorted.sort_by_key(|c| c.priority);
        sorted
    }

    /// Get owned commands, sorted by priority.
    pub fn into_commands(mut self) -> Vec<TransactionalCommand> {
        self.commands.sort_by_key(|c| c.priority);
        self.commands
    }

    /// Get number of commands in batch.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Check if batch is empty.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Set whether to validate topology after batch.
    pub fn validate_after(mut self, validate: bool) -> Self {
        self.validate_topology = validate;
        self
    }

    /// Set whether to send confirmation event.
    pub fn confirm_after(mut self, confirm: bool) -> Self {
        self.send_confirmation = confirm;
        self
    }

    /// Generate rollback commands for this batch.
    pub fn rollback_commands(&self) -> Vec<EngineCommand> {
        self.commands
            .iter()
            .rev() // Reverse order for rollback
            .filter_map(|c| c.reverse.as_ref().map(|r| (**r).clone()))
            .collect()
    }

    /// Check if this batch can be fully rolled back.
    pub fn is_fully_reversible(&self) -> bool {
        self.commands.iter().all(|c| c.reversible)
    }

    /// Extract just the EngineCommands (without metadata), sorted.
    pub fn extract_commands(&self) -> Vec<EngineCommand> {
        self.commands()
            .into_iter()
            .map(|c| c.command.clone())
            .collect()
    }
}

/// Builder for creating common batch patterns.
pub struct BatchBuilder {
    batch: CommandBatch,
}

impl BatchBuilder {
    /// Start building a new batch.
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            batch: CommandBatch::new(description),
        }
    }

    /// Start building a patch load batch.
    pub fn patch_load(name: impl Into<String>) -> Self {
        Self {
            batch: CommandBatch::for_patch_load(name),
        }
    }

    /// Clear all modules first.
    pub fn clear_all(mut self) -> Self {
        self.batch
            .add_with_priority(EngineCommand::ClearAllModules, 1);
        self
    }

    /// Add an oscillator module.
    pub fn oscillator(mut self, instance: u16) -> Self {
        use crate::modules::Oscillator;
        let id = ModuleId::new(ModuleType::Oscillator, instance);
        self.batch.add_module(id, Box::new(Oscillator::new()));
        self
    }

    /// Add a filter module.
    pub fn filter(mut self, instance: u16) -> Self {
        use crate::modules::Filter;
        let id = ModuleId::new(ModuleType::Filter, instance);
        self.batch.add_module(id, Box::new(Filter::new()));
        self
    }

    /// Add an envelope module.
    pub fn envelope(mut self, instance: u16) -> Self {
        use crate::modules::Envelope;
        let id = ModuleId::new(ModuleType::Envelope, instance);
        self.batch.add_module(id, Box::new(Envelope::new()));
        self
    }

    /// Connect two ports.
    pub fn connect(mut self, from: PortId, to: PortId) -> Self {
        self.batch.add_connection(from, to);
        self
    }

    /// Set a parameter.
    /// The Param contains both the parameter type and its value.
    pub fn param(mut self, module_id: ModuleId, param: Param) -> Self {
        self.batch.set_parameter(module_id, param);
        self
    }

    /// Build the batch.
    pub fn build(self) -> CommandBatch {
        self.batch
    }
}

/// Result of executing a batch.
#[derive(Debug)]
pub struct BatchResult {
    /// Transaction ID.
    pub id: TransactionId,
    /// Whether all commands succeeded.
    pub success: bool,
    /// Number of commands executed successfully.
    pub commands_executed: usize,
    /// Error message if failed.
    pub error: Option<String>,
    /// Commands that were rolled back (if any).
    pub rolled_back: usize,
}

impl BatchResult {
    /// Create a successful result.
    pub fn success(id: TransactionId, commands_executed: usize) -> Self {
        Self {
            id,
            success: true,
            commands_executed,
            error: None,
            rolled_back: 0,
        }
    }

    /// Create a failed result.
    pub fn failure(
        id: TransactionId,
        commands_executed: usize,
        error: String,
        rolled_back: usize,
    ) -> Self {
        Self {
            id,
            success: false,
            commands_executed,
            error: Some(error),
            rolled_back,
        }
    }
}

// We need to implement Clone for EngineCommand for rollback support
// This is a workaround since EngineCommand contains Box<dyn PolyModule>
impl Clone for EngineCommand {
    fn clone(&self) -> Self {
        match self {
            // Commands that can be cloned
            EngineCommand::NoteOn {
                note,
                velocity,
                channel,
            } => EngineCommand::NoteOn {
                note: *note,
                velocity: *velocity,
                channel: *channel,
            },
            EngineCommand::NoteOff { note, channel } => EngineCommand::NoteOff {
                note: *note,
                channel: *channel,
            },
            EngineCommand::AllNotesOff => EngineCommand::AllNotesOff,
            EngineCommand::PitchBend { value, channel } => EngineCommand::PitchBend {
                value: *value,
                channel: *channel,
            },
            EngineCommand::ModWheel { value, channel } => EngineCommand::ModWheel {
                value: *value,
                channel: *channel,
            },
            EngineCommand::Aftertouch { value, channel } => EngineCommand::Aftertouch {
                value: *value,
                channel: *channel,
            },
            EngineCommand::PolyAftertouch {
                note,
                value,
                channel,
            } => EngineCommand::PolyAftertouch {
                note: *note,
                value: *value,
                channel: *channel,
            },
            EngineCommand::SetVoiceParameter {
                instrument_id,
                target,
                param,
            } => EngineCommand::SetVoiceParameter {
                instrument_id: *instrument_id,
                target: *target,
                param: *param,
            },
            EngineCommand::SetModuleParameter {
                instrument_id,
                module_id,
                param,
            } => EngineCommand::SetModuleParameter {
                instrument_id: *instrument_id,
                module_id: *module_id,
                param: *param,
            },
            EngineCommand::RemoveModule { instrument_id, id } => EngineCommand::RemoveModule {
                instrument_id: *instrument_id,
                id: *id,
            },
            EngineCommand::Connect {
                instrument_id,
                from,
                to,
            } => EngineCommand::Connect {
                instrument_id: *instrument_id,
                from: from.clone(),
                to: to.clone(),
            },
            EngineCommand::Disconnect {
                instrument_id,
                from,
                to,
            } => EngineCommand::Disconnect {
                instrument_id: *instrument_id,
                from: from.clone(),
                to: to.clone(),
            },
            EngineCommand::DisconnectAll {
                instrument_id,
                module,
            } => EngineCommand::DisconnectAll {
                instrument_id: *instrument_id,
                module: *module,
            },
            EngineCommand::SetTempo(t) => EngineCommand::SetTempo(*t),
            EngineCommand::Play => EngineCommand::Play,
            EngineCommand::Stop => EngineCommand::Stop,
            EngineCommand::Pause => EngineCommand::Pause,
            EngineCommand::Rewind => EngineCommand::Rewind,
            EngineCommand::Reset => EngineCommand::Reset,
            EngineCommand::ClearAllModules => EngineCommand::ClearAllModules,
            EngineCommand::SetMasterVolume(v) => EngineCommand::SetMasterVolume(*v),
            EngineCommand::SetGlideTime(t) => EngineCommand::SetGlideTime(*t),
            EngineCommand::SetBypass { module, bypass } => EngineCommand::SetBypass {
                module: *module,
                bypass: *bypass,
            },
            EngineCommand::RemoveVisualizer { instrument_id, id } => {
                EngineCommand::RemoveVisualizer {
                    instrument_id: *instrument_id,
                    id: *id,
                }
            }
            EngineCommand::RemoveEffect { instrument_id, id } => EngineCommand::RemoveEffect {
                instrument_id: *instrument_id,
                id: *id,
            },
            EngineCommand::SetEffectParameter {
                instrument_id,
                effect_type,
                param,
            } => EngineCommand::SetEffectParameter {
                instrument_id: *instrument_id,
                effect_type: *effect_type,
                param: *param,
            },
            EngineCommand::SetEffectEnabled {
                instrument_id,
                effect_type,
                enabled,
            } => EngineCommand::SetEffectEnabled {
                instrument_id: *instrument_id,
                effect_type: *effect_type,
                enabled: *enabled,
            },
            // Instrument management commands
            EngineCommand::RemoveInstrument { instrument_id } => EngineCommand::RemoveInstrument {
                instrument_id: *instrument_id,
            },
            EngineCommand::SetInstrumentParameter {
                instrument_id,
                param,
            } => EngineCommand::SetInstrumentParameter {
                instrument_id: *instrument_id,
                param: *param,
            },
            EngineCommand::SetInstrumentMidiChannel {
                instrument_id,
                channel,
            } => EngineCommand::SetInstrumentMidiChannel {
                instrument_id: *instrument_id,
                channel: *channel,
            },
            EngineCommand::SetInstrumentEnabled {
                instrument_id,
                enabled,
            } => EngineCommand::SetInstrumentEnabled {
                instrument_id: *instrument_id,
                enabled: *enabled,
            },
            // Commands with Box<dyn ...> cannot be cloned - panic if attempted
            EngineCommand::AddInstrument { .. } => {
                panic!("AddInstrument cannot be cloned - instrument instances are unique")
            }
            EngineCommand::AddModuleInstance { .. } => {
                panic!("AddModuleInstance cannot be cloned - module instances are unique")
            }
            EngineCommand::AddEffectInstance { .. } => {
                panic!("AddEffectInstance cannot be cloned - effect instances are unique")
            }
            EngineCommand::AddVisualizer { .. } => {
                panic!("AddVisualizer cannot be cloned - visualizer buffers are shared")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_id() {
        let id1 = TransactionId::new();
        let id2 = TransactionId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_command_batch() {
        let mut batch = CommandBatch::new("Test batch");

        batch
            .add(EngineCommand::SetMasterVolume(crate::types::Gain::new(0.8)))
            .add(EngineCommand::SetTempo(crate::types::Bpm::new(120.0)));

        assert_eq!(batch.len(), 2);
        assert!(!batch.is_empty());
    }

    #[test]
    fn test_batch_priority_ordering() {
        let mut batch = CommandBatch::new("Priority test");

        // Add in wrong order
        batch.add_with_priority(
            EngineCommand::SetMasterVolume(crate::types::Gain::new(1.0)),
            100,
        );
        batch.add_with_priority(EngineCommand::ClearAllModules, 1);
        batch.add_with_priority(EngineCommand::SetTempo(crate::types::Bpm::new(120.0)), 50);

        let commands = batch.commands();

        // Should be sorted by priority
        assert_eq!(commands[0].priority, 1);
        assert_eq!(commands[1].priority, 50);
        assert_eq!(commands[2].priority, 100);
    }

    #[test]
    fn test_reversible_commands() {
        let mut batch = CommandBatch::new("Reversible test");

        batch.add_reversible(
            EngineCommand::SetMasterVolume(crate::types::Gain::new(0.8)),
            EngineCommand::SetMasterVolume(crate::types::Gain::new(1.0)), // Reverse to original
        );

        assert!(batch.commands[0].reversible);
        assert!(batch.commands[0].reverse.is_some());

        let rollback = batch.rollback_commands();
        assert_eq!(rollback.len(), 1);
    }

    #[test]
    fn test_batch_builder() {
        let batch = BatchBuilder::new("Builder test").clear_all().build();

        assert!(!batch.is_empty());
        let commands = batch.commands();
        assert!(matches!(
            commands[0].command,
            EngineCommand::ClearAllModules
        ));
    }

    #[test]
    fn test_batch_result() {
        let id = TransactionId::new();

        let success = BatchResult::success(id, 5);
        assert!(success.success);
        assert_eq!(success.commands_executed, 5);

        let failure = BatchResult::failure(id, 3, "Test error".to_string(), 2);
        assert!(!failure.success);
        assert_eq!(failure.commands_executed, 3);
        assert_eq!(failure.rolled_back, 2);
    }
}
