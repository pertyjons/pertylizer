//! Transactional command batching for atomic operations.
//!
//! This module provides support for grouping multiple engine commands
//! into atomic batches that succeed or fail together, useful for
//! patch loading and complex multi-module operations.

use std::sync::atomic::{AtomicU64, Ordering};

use super::commands::{EngineCommand, ModuleId, PortId};
use super::instrument::InstrumentId;
use synth_core::{ModuleType, Param};

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
        module: Box<dyn synth_core::PolyModule>,
    ) -> &mut Self {
        self.add_module_to(None, id, module)
    }

    /// Add a module to a specific instrument's voice graph or global graph.
    pub fn add_module_to(
        &mut self,
        instrument_id: Option<InstrumentId>,
        id: ModuleId,
        module: Box<dyn synth_core::PolyModule>,
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
                from,
                to,
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
    ///
    /// Only reversible commands contribute to rollback. Reverse commands are
    /// always clonable types (e.g., `RemoveModule`, `Disconnect`), so
    /// `try_clone()` will always return `Some` here.
    pub fn rollback_commands(&self) -> Vec<EngineCommand> {
        self.commands
            .iter()
            .rev() // Reverse order for rollback
            .filter_map(|c| c.reverse.as_ref().and_then(|r| r.try_clone()))
            .collect()
    }

    /// Check if this batch can be fully rolled back.
    pub fn is_fully_reversible(&self) -> bool {
        self.commands.iter().all(|c| c.reversible)
    }

    /// Extract just the clonable EngineCommands (without metadata), sorted.
    ///
    /// Commands containing unique owned resources (e.g., `AddModuleInstance`,
    /// `AddInstrument`) are skipped. Use `into_commands()` to consume the
    /// batch and get all commands including non-clonable ones.
    pub fn extract_commands(&self) -> Vec<EngineCommand> {
        self.commands()
            .into_iter()
            .filter_map(|c| c.command.try_clone())
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
        use synth_modules::Oscillator;
        let id = ModuleId::new(ModuleType::Oscillator, instance);
        self.batch.add_module(id, Box::new(Oscillator::new()));
        self
    }

    /// Add a filter module.
    pub fn filter(mut self, instance: u16) -> Self {
        use synth_modules::Filter;
        let id = ModuleId::new(ModuleType::Filter, instance);
        self.batch.add_module(id, Box::new(Filter::new()));
        self
    }

    /// Add an envelope module.
    pub fn envelope(mut self, instance: u16) -> Self {
        use synth_modules::Envelope;
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

impl EngineCommand {
    /// Try to clone this command. Returns `None` for commands that contain
    /// unique owned resources (`Box<dyn PolyModule>`, `Box<dyn AudioEffect>`,
    /// `Box<Instrument>`, `Arc<VisualizationBuffer>`) which cannot be duplicated.
    ///
    /// Used by `CommandBatch::rollback_commands()` and `extract_commands()`.
    /// Reverse commands (e.g., `RemoveModule`, `Disconnect`) are always clonable.
    pub fn try_clone(&self) -> Option<Self> {
        Some(match self {
            // Commands that can be cloned
            Self::NoteOn {
                note,
                velocity,
                channel,
            } => Self::NoteOn {
                note: *note,
                velocity: *velocity,
                channel: *channel,
            },
            Self::NoteOff { note, channel } => Self::NoteOff {
                note: *note,
                channel: *channel,
            },
            Self::AllNotesOff => Self::AllNotesOff,
            Self::PitchBend { value, channel } => Self::PitchBend {
                value: *value,
                channel: *channel,
            },
            Self::ModWheel { value, channel } => Self::ModWheel {
                value: *value,
                channel: *channel,
            },
            Self::Aftertouch { value, channel } => Self::Aftertouch {
                value: *value,
                channel: *channel,
            },
            Self::PolyAftertouch {
                note,
                value,
                channel,
            } => Self::PolyAftertouch {
                note: *note,
                value: *value,
                channel: *channel,
            },
            Self::SetVoiceParameter {
                instrument_id,
                target,
                param,
            } => Self::SetVoiceParameter {
                instrument_id: *instrument_id,
                target: *target,
                param: *param,
            },
            Self::SetModuleParameter {
                instrument_id,
                module_id,
                param,
            } => Self::SetModuleParameter {
                instrument_id: *instrument_id,
                module_id: *module_id,
                param: *param,
            },
            // The script is a shared `Arc` — cloning bumps the refcount.
            Self::SetModScript {
                instrument_id,
                module_id,
                slot,
                script,
            } => Self::SetModScript {
                instrument_id: *instrument_id,
                module_id: *module_id,
                slot: *slot,
                script: script.clone(),
            },
            Self::RemoveModule { instrument_id, id } => Self::RemoveModule {
                instrument_id: *instrument_id,
                id: *id,
            },
            Self::Connect {
                instrument_id,
                from,
                to,
            } => Self::Connect {
                instrument_id: *instrument_id,
                from: *from,
                to: *to,
            },
            Self::Disconnect {
                instrument_id,
                from,
                to,
            } => Self::Disconnect {
                instrument_id: *instrument_id,
                from: *from,
                to: *to,
            },
            Self::DisconnectAll {
                instrument_id,
                module,
            } => Self::DisconnectAll {
                instrument_id: *instrument_id,
                module: *module,
            },
            Self::SetTempo(t) => Self::SetTempo(*t),
            Self::Play => Self::Play,
            Self::Stop => Self::Stop,
            Self::Pause => Self::Pause,
            Self::Rewind => Self::Rewind,
            Self::Seek { tick } => Self::Seek { tick: *tick },
            Self::PlayPattern {
                pattern_id,
                instrument,
            } => Self::PlayPattern {
                pattern_id: *pattern_id,
                instrument: *instrument,
            },
            Self::PlayFromPattern { pattern_id } => Self::PlayFromPattern {
                pattern_id: *pattern_id,
            },
            Self::SetSoloPattern(p) => Self::SetSoloPattern(*p),
            Self::SetPreviewPattern(p) => Self::SetPreviewPattern(*p),
            Self::SetLoop {
                start,
                end,
                enabled,
            } => Self::SetLoop {
                start: *start,
                end: *end,
                enabled: *enabled,
            },
            Self::SetRepeat { enabled } => Self::SetRepeat { enabled: *enabled },
            Self::Reset => Self::Reset,
            Self::ClearAllModules => Self::ClearAllModules,
            Self::SetMasterVolume(v) => Self::SetMasterVolume(*v),
            Self::SetGlideTime(t) => Self::SetGlideTime(*t),
            Self::SetFocusedInstrument(id) => Self::SetFocusedInstrument(*id),
            Self::SetBypass {
                instrument_id,
                module,
                bypass,
            } => Self::SetBypass {
                instrument_id: *instrument_id,
                module: *module,
                bypass: *bypass,
            },
            Self::RemoveVisualizer { instrument_id, id } => Self::RemoveVisualizer {
                instrument_id: *instrument_id,
                id: *id,
            },
            Self::RemoveEffect { instrument_id, id } => Self::RemoveEffect {
                instrument_id: *instrument_id,
                id: *id,
            },
            Self::SetEffectParameter {
                instrument_id,
                module_id,
                param,
            } => Self::SetEffectParameter {
                instrument_id: *instrument_id,
                module_id: *module_id,
                param: *param,
            },
            Self::SetEffectEnabled {
                instrument_id,
                module_id,
                enabled,
            } => Self::SetEffectEnabled {
                instrument_id: *instrument_id,
                module_id: *module_id,
                enabled: *enabled,
            },
            // Instrument management commands
            Self::RemoveInstrument { instrument_id } => Self::RemoveInstrument {
                instrument_id: *instrument_id,
            },
            Self::RenameInstrument {
                instrument_id,
                name,
            } => Self::RenameInstrument {
                instrument_id: *instrument_id,
                name: name.clone(),
            },
            Self::SetInstrumentDescription {
                instrument_id,
                description,
            } => Self::SetInstrumentDescription {
                instrument_id: *instrument_id,
                description: description.clone(),
            },
            Self::SetInstrumentColor {
                instrument_id,
                color,
            } => Self::SetInstrumentColor {
                instrument_id: *instrument_id,
                color: color.clone(),
            },
            Self::SetPatchDescription {
                instrument_id,
                description,
            } => Self::SetPatchDescription {
                instrument_id: *instrument_id,
                description: description.clone(),
            },
            Self::SetModuleDescription {
                instrument_id,
                module_id,
                description,
            } => Self::SetModuleDescription {
                instrument_id: *instrument_id,
                module_id: *module_id,
                description: description.clone(),
            },
            Self::SetSidechainSource {
                instrument_id,
                source,
            } => Self::SetSidechainSource {
                instrument_id: *instrument_id,
                source: *source,
            },
            Self::SetInstrumentParameter {
                instrument_id,
                param,
            } => Self::SetInstrumentParameter {
                instrument_id: *instrument_id,
                param: *param,
            },
            Self::SetInstrumentMidiChannel {
                instrument_id,
                channel,
            } => Self::SetInstrumentMidiChannel {
                instrument_id: *instrument_id,
                channel: *channel,
            },
            Self::SetInstrumentEnabled {
                instrument_id,
                enabled,
            } => Self::SetInstrumentEnabled {
                instrument_id: *instrument_id,
                enabled: *enabled,
            },
            Self::SetInstrumentSolo {
                instrument_id,
                solo,
            } => Self::SetInstrumentSolo {
                instrument_id: *instrument_id,
                solo: *solo,
            },
            Self::SetInstrumentCategory {
                instrument_id,
                category,
            } => Self::SetInstrumentCategory {
                instrument_id: *instrument_id,
                category: *category,
            },
            // Song (Arc can be cloned)
            Self::SetSong { song } => Self::SetSong {
                song: std::sync::Arc::clone(song),
            },
            // Commands with unique owned resources cannot be cloned
            Self::AddInstrument { .. }
            | Self::AddModuleInstance { .. }
            | Self::AddEffectInstance { .. }
            | Self::AddReturnEffect { .. }
            | Self::AddVisualizer { .. } => return None,
            // AWE commands
            Self::SetAweParameter { param } => Self::SetAweParameter { param: *param },
            Self::SetAweEnabled { enabled } => Self::SetAweEnabled { enabled: *enabled },
            Self::SetAweState { snapshot } => Self::SetAweState {
                snapshot: *snapshot,
            },

            // Recording commands
            Self::ArmRecord {
                pattern_id,
                track_id,
                region_start,
                pattern_length,
                ticks_per_bar,
                quantize_grid,
                overdub,
            } => Self::ArmRecord {
                pattern_id: *pattern_id,
                track_id: *track_id,
                region_start: *region_start,
                pattern_length: *pattern_length,
                ticks_per_bar: *ticks_per_bar,
                quantize_grid: *quantize_grid,
                overdub: *overdub,
            },
            Self::DisarmRecord => Self::DisarmRecord,
            Self::SetMetronome(enabled) => Self::SetMetronome(*enabled),
            Self::SetMetronomeVolume(vol) => Self::SetMetronomeVolume(*vol),
            Self::ReorderEffect {
                instrument_id,
                module_id,
                direction,
            } => Self::ReorderEffect {
                instrument_id: *instrument_id,
                module_id: *module_id,
                direction: *direction,
            },
            Self::SetEffectChainOrder {
                instrument_id,
                order,
            } => Self::SetEffectChainOrder {
                instrument_id: *instrument_id,
                order: order.clone(),
            },

            // Return-bus commands (all clonable)
            Self::CreateReturnBus { id } => Self::CreateReturnBus { id: *id },
            Self::RemoveReturnBus { id } => Self::RemoveReturnBus { id: *id },
            Self::ClearReturnBusses => Self::ClearReturnBusses,
            Self::ClearMasterEffects => Self::ClearMasterEffects,
            Self::RemoveReturnEffect { return_id, id } => Self::RemoveReturnEffect {
                return_id: *return_id,
                id: *id,
            },
            Self::SetReturnEffectParameter {
                return_id,
                module_id,
                param,
            } => Self::SetReturnEffectParameter {
                return_id: *return_id,
                module_id: *module_id,
                param: *param,
            },
            Self::SetReturnEffectEnabled {
                return_id,
                module_id,
                enabled,
            } => Self::SetReturnEffectEnabled {
                return_id: *return_id,
                module_id: *module_id,
                enabled: *enabled,
            },
            Self::ReorderReturnEffect {
                return_id,
                module_id,
                direction,
            } => Self::ReorderReturnEffect {
                return_id: *return_id,
                module_id: *module_id,
                direction: *direction,
            },

            // Non-clonable commands (contain move-only types or Arc data)
            Self::SetAudioInputConsumer { .. }
            | Self::ClearAudioInputConsumer
            | Self::LoadSampleData { .. } => {
                return None;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::assert_matches;

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
            .add(EngineCommand::SetMasterVolume(synth_core::Gain::new(0.8)))
            .add(EngineCommand::SetTempo(synth_core::Bpm::new(120.0)));

        assert_eq!(batch.len(), 2);
        assert!(!batch.is_empty());
    }

    #[test]
    fn test_batch_priority_ordering() {
        let mut batch = CommandBatch::new("Priority test");

        // Add in wrong order
        batch.add_with_priority(
            EngineCommand::SetMasterVolume(synth_core::Gain::new(1.0)),
            100,
        );
        batch.add_with_priority(EngineCommand::ClearAllModules, 1);
        batch.add_with_priority(EngineCommand::SetTempo(synth_core::Bpm::new(120.0)), 50);

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
            EngineCommand::SetMasterVolume(synth_core::Gain::new(0.8)),
            EngineCommand::SetMasterVolume(synth_core::Gain::new(1.0)), // Reverse to original
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
        assert_matches!(commands[0].command, EngineCommand::ClearAllModules);
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
