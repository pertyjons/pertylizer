//! Multi-GUI engine hub for concurrent client access.
//!
//! This module provides the infrastructure for multiple GUI clients to
//! connect to a single engine instance, with proper access control and
//! event distribution.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use parking_lot::{Mutex, RwLock};
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use thiserror::Error;

use super::commands::{EngineCommand, ModuleId};
use super::event_priority::{EventPriority, TimestampedEvent};
use super::shared_state::SharedEngineState;

/// Unique identifier for a connected client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientId(pub u64);

impl ClientId {
    /// The primary/local GUI client ID.
    pub const PRIMARY: Self = Self(0);
}

/// Type of connected client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientType {
    /// Local GUI (primary control).
    LocalGui,
    /// Remote GUI over network.
    RemoteGui,
    /// Automation/scripting client.
    Automation,
    /// Read-only observer (meters only).
    Observer,
}

/// Permissions for a client.
#[derive(Debug, Clone)]
pub struct ClientPermissions {
    /// Can send parameter changes.
    pub can_modify_params: bool,
    /// Can add/remove modules.
    pub can_modify_topology: bool,
    /// Can control transport (play/stop).
    pub can_control_transport: bool,
    /// Can receive visualization data.
    pub receives_visualization: bool,
    /// Can receive full state updates.
    pub receives_state_updates: bool,
    /// Priority level for conflicting operations.
    pub priority: u8,
}

impl ClientPermissions {
    /// Full control permissions (for primary local GUI).
    pub fn full_control() -> Self {
        Self {
            can_modify_params: true,
            can_modify_topology: true,
            can_control_transport: true,
            receives_visualization: true,
            receives_state_updates: true,
            priority: 255,
        }
    }

    /// Read-only permissions (for observers).
    pub fn read_only() -> Self {
        Self {
            can_modify_params: false,
            can_modify_topology: false,
            can_control_transport: false,
            receives_visualization: true,
            receives_state_updates: true,
            priority: 0,
        }
    }

    /// Remote GUI permissions (can modify but lower priority).
    pub fn remote_gui() -> Self {
        Self {
            can_modify_params: true,
            can_modify_topology: true,
            can_control_transport: true,
            receives_visualization: true,
            receives_state_updates: true,
            priority: 128,
        }
    }

    /// Automation permissions (params only).
    pub fn automation() -> Self {
        Self {
            can_modify_params: true,
            can_modify_topology: false,
            can_control_transport: false,
            receives_visualization: false,
            receives_state_updates: false,
            priority: 64,
        }
    }
}

impl Default for ClientPermissions {
    fn default() -> Self {
        Self::read_only()
    }
}

/// Information about a connected client.
struct ClientInfo {
    /// Client type.
    client_type: ClientType,
    /// Client permissions.
    permissions: ClientPermissions,
    /// Event producer for this client.
    event_producer: Mutex<ringbuf::HeapProd<TimestampedEvent>>,
    /// Whether client is still connected.
    connected: AtomicBool,
    /// Last activity timestamp (sample position).
    last_activity: AtomicU64,
    /// Modules this client is "focused" on (receives detailed updates).
    focused_modules: RwLock<Vec<ModuleId>>,
}

impl std::fmt::Debug for ClientInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientInfo")
            .field("client_type", &self.client_type)
            .field("permissions", &self.permissions)
            .field("event_producer", &"<ringbuf::HeapProd>")
            .field("connected", &self.connected.load(Ordering::Relaxed))
            .field("last_activity", &self.last_activity.load(Ordering::Relaxed))
            .field("focused_modules", &*self.focused_modules.read())
            .finish()
    }
}

/// Buffer sizes for client event channels.
const CLIENT_EVENT_BUFFER_SIZE: usize = 1024;

/// Central hub for multi-GUI access to the engine.
///
/// The hub manages:
/// - Client registration and permissions
/// - Event distribution to all clients
/// - Command routing with conflict resolution
/// - Shared state access
pub struct EngineHub {
    /// Connected clients.
    clients: RwLock<HashMap<ClientId, Arc<ClientInfo>>>,
    /// Next client ID to assign.
    next_client_id: AtomicU64,
    /// Shared engine state (meters, transport, graph).
    shared_state: Arc<SharedEngineState>,
    /// Command producer to engine (from hub).
    command_producer: Mutex<ringbuf::HeapProd<EngineCommand>>,
    /// Current timestamp for events.
    current_timestamp: AtomicU64,
    /// Whether any client has exclusive control.
    exclusive_client: RwLock<Option<ClientId>>,
}

impl EngineHub {
    /// Create a new engine hub.
    pub fn new(
        shared_state: Arc<SharedEngineState>,
        command_producer: ringbuf::HeapProd<EngineCommand>,
    ) -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
            next_client_id: AtomicU64::new(1), // 0 is reserved for PRIMARY
            shared_state,
            command_producer: Mutex::new(command_producer),
            current_timestamp: AtomicU64::new(0),
            exclusive_client: RwLock::new(None),
        }
    }

    /// Register the primary (local) GUI client.
    pub fn register_primary(&self) -> ClientHandle {
        self.register_client_internal(
            ClientId::PRIMARY,
            ClientType::LocalGui,
            ClientPermissions::full_control(),
        )
    }

    /// Register a new client and get a handle.
    pub fn register_client(
        &self,
        client_type: ClientType,
        permissions: ClientPermissions,
    ) -> ClientHandle {
        let id = ClientId(self.next_client_id.fetch_add(1, Ordering::Relaxed));
        self.register_client_internal(id, client_type, permissions)
    }

    fn register_client_internal(
        &self,
        id: ClientId,
        client_type: ClientType,
        permissions: ClientPermissions,
    ) -> ClientHandle {
        // Create event channel for this client
        let event_rb = HeapRb::<TimestampedEvent>::new(CLIENT_EVENT_BUFFER_SIZE);
        let (event_prod, event_cons) = event_rb.split();

        let client_info = Arc::new(ClientInfo {
            client_type,
            permissions: permissions.clone(),
            event_producer: Mutex::new(event_prod),
            connected: AtomicBool::new(true),
            last_activity: AtomicU64::new(0),
            focused_modules: RwLock::new(Vec::new()),
        });

        self.clients.write().insert(id, client_info.clone());

        ClientHandle {
            id,
            permissions,
            event_consumer: Mutex::new(event_cons),
            shared_state: self.shared_state.clone(),
            client_info,
        }
    }

    /// Unregister a client.
    pub fn unregister_client(&self, id: ClientId) {
        if let Some(client) = self.clients.write().remove(&id) {
            client.connected.store(false, Ordering::Release);
        }

        // Clear exclusive control if this client had it
        let mut exclusive = self.exclusive_client.write();
        if *exclusive == Some(id) {
            *exclusive = None;
        }
    }

    /// Broadcast an event to all connected clients.
    pub fn broadcast_event(&self, event: TimestampedEvent) {
        let clients = self.clients.read();

        for (_, client) in clients.iter() {
            if !client.connected.load(Ordering::Acquire) {
                continue;
            }

            // Check if client should receive this type of event
            let should_receive = match event.priority {
                EventPriority::Low => client.permissions.receives_visualization,
                _ => client.permissions.receives_state_updates,
            };

            if should_receive {
                let mut producer = client.event_producer.lock();
                // Best effort - drop if buffer full
                let _ = producer.try_push(event.clone());
            }
        }
    }

    /// Send a command to the engine (with permission check).
    pub fn send_command(
        &self,
        client_id: ClientId,
        command: EngineCommand,
    ) -> Result<(), HubError> {
        // Check if another client has exclusive control
        if let Some(exclusive_id) = *self.exclusive_client.read()
            && exclusive_id != client_id
        {
            return Err(HubError::ExclusiveControlActive);
        }

        // Get client and check permissions
        let clients = self.clients.read();
        let client = clients.get(&client_id).ok_or(HubError::ClientNotFound)?;

        if !client.connected.load(Ordering::Acquire) {
            return Err(HubError::ClientDisconnected);
        }

        // Check permission for this command type
        if !self.check_command_permission(&client.permissions, &command) {
            return Err(HubError::PermissionDenied);
        }

        // Update activity timestamp
        client.last_activity.store(
            self.current_timestamp.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );

        // Send to engine
        let mut producer = self.command_producer.lock();
        producer
            .try_push(command)
            .map_err(|_| HubError::CommandBufferFull)?;

        Ok(())
    }

    fn check_command_permission(&self, perms: &ClientPermissions, command: &EngineCommand) -> bool {
        match command {
            // Instrument topology changes
            EngineCommand::AddInstrument { .. } | EngineCommand::RemoveInstrument { .. } => {
                perms.can_modify_topology
            }

            // Instrument parameter changes
            EngineCommand::SetInstrumentParameter { .. }
            | EngineCommand::SetInstrumentMidiChannel { .. }
            | EngineCommand::SetInstrumentEnabled { .. }
            | EngineCommand::SetInstrumentSolo { .. } => perms.can_modify_params,

            // Parameter changes
            EngineCommand::SetVoiceParameter { .. }
            | EngineCommand::SetModuleParameter { .. }
            | EngineCommand::SetEffectParameter { .. }
            | EngineCommand::SetMasterVolume(_)
            | EngineCommand::SetGlideTime(_)
            | EngineCommand::SetFocusedInstrument(_) => perms.can_modify_params,

            // Topology changes
            EngineCommand::AddModuleInstance { .. }
            | EngineCommand::RemoveModule { .. }
            | EngineCommand::Connect { .. }
            | EngineCommand::Disconnect { .. }
            | EngineCommand::DisconnectAll { .. }
            | EngineCommand::AddEffectInstance { .. }
            | EngineCommand::RemoveEffect { .. }
            | EngineCommand::AddVisualizer { .. }
            | EngineCommand::RemoveVisualizer { .. }
            | EngineCommand::ClearAllModules => perms.can_modify_topology,

            // Transport control
            EngineCommand::Play
            | EngineCommand::Stop
            | EngineCommand::Pause
            | EngineCommand::Rewind
            | EngineCommand::SetSoloTrack(_)
            | EngineCommand::SetTempo(_)
            | EngineCommand::SetSong { .. }
            | EngineCommand::Seek { .. }
            | EngineCommand::PlayPattern { .. }
            | EngineCommand::PlayFromPattern { .. } => perms.can_control_transport,

            // Note events - require param permission
            EngineCommand::NoteOn { .. }
            | EngineCommand::NoteOff { .. }
            | EngineCommand::AllNotesOff
            | EngineCommand::PitchBend { .. }
            | EngineCommand::ModWheel { .. }
            | EngineCommand::Aftertouch { .. }
            | EngineCommand::PolyAftertouch { .. } => perms.can_modify_params,

            // Engine control
            EngineCommand::Reset => perms.can_modify_topology,
            EngineCommand::SetBypass { .. } => perms.can_modify_params,
            EngineCommand::SetEffectEnabled { .. } => perms.can_modify_params,

            // Sample loading
            EngineCommand::LoadSample { .. } => perms.can_modify_params,
        }
    }

    /// Request exclusive control for a client.
    pub fn request_exclusive(&self, client_id: ClientId) -> Result<(), HubError> {
        let clients = self.clients.read();
        let client = clients.get(&client_id).ok_or(HubError::ClientNotFound)?;

        // Only high-priority clients can request exclusive
        if client.permissions.priority < 128 {
            return Err(HubError::PermissionDenied);
        }

        let mut exclusive = self.exclusive_client.write();
        if exclusive.is_some() {
            return Err(HubError::ExclusiveControlActive);
        }

        *exclusive = Some(client_id);
        Ok(())
    }

    /// Release exclusive control.
    pub fn release_exclusive(&self, client_id: ClientId) {
        let mut exclusive = self.exclusive_client.write();
        if *exclusive == Some(client_id) {
            *exclusive = None;
        }
    }

    /// Get the shared engine state.
    pub fn shared_state(&self) -> &Arc<SharedEngineState> {
        &self.shared_state
    }

    /// Update the current timestamp (called from engine).
    pub fn set_timestamp(&self, timestamp: u64) {
        self.current_timestamp.store(timestamp, Ordering::Relaxed);
    }

    /// Get count of connected clients.
    pub fn client_count(&self) -> usize {
        self.clients
            .read()
            .values()
            .filter(|c| c.connected.load(Ordering::Relaxed))
            .count()
    }

    /// Clean up disconnected clients.
    pub fn cleanup_disconnected(&self) {
        let mut clients = self.clients.write();
        clients.retain(|_, c| c.connected.load(Ordering::Relaxed));
    }
}

/// Handle for a connected client to interact with the engine.
pub struct ClientHandle {
    /// This client's ID.
    pub id: ClientId,
    /// This client's permissions.
    pub permissions: ClientPermissions,
    /// Event consumer for receiving engine events.
    event_consumer: Mutex<ringbuf::HeapCons<TimestampedEvent>>,
    /// Reference to shared state.
    shared_state: Arc<SharedEngineState>,
    /// Reference to client info (for updating focus, etc.).
    client_info: Arc<ClientInfo>,
}

impl ClientHandle {
    /// Poll for the next event.
    pub fn poll_event(&self) -> Option<TimestampedEvent> {
        self.event_consumer.lock().try_pop()
    }

    /// Poll all available events.
    pub fn poll_all_events(&self) -> Vec<TimestampedEvent> {
        let mut events = Vec::new();
        let mut consumer = self.event_consumer.lock();
        while let Some(event) = consumer.try_pop() {
            events.push(event);
        }
        events
    }

    /// Check if there are pending events.
    pub fn has_events(&self) -> bool {
        !self.event_consumer.lock().is_empty()
    }

    /// Get reference to shared state.
    pub fn shared_state(&self) -> &SharedEngineState {
        &self.shared_state
    }

    /// Set which modules this client is focused on (for detailed updates).
    pub fn set_focused_modules(&self, modules: Vec<ModuleId>) {
        *self.client_info.focused_modules.write() = modules;
    }

    /// Add a module to the focus list.
    pub fn add_focused_module(&self, module: ModuleId) {
        let mut focused = self.client_info.focused_modules.write();
        if !focused.contains(&module) {
            focused.push(module);
        }
    }

    /// Remove a module from the focus list.
    pub fn remove_focused_module(&self, module: ModuleId) {
        self.client_info
            .focused_modules
            .write()
            .retain(|m| *m != module);
    }

    /// Get the focused modules.
    pub fn focused_modules(&self) -> Vec<ModuleId> {
        self.client_info.focused_modules.read().clone()
    }

    /// Mark this client as disconnected.
    pub fn disconnect(&self) {
        self.client_info.connected.store(false, Ordering::Release);
    }

    /// Check if still connected.
    pub fn is_connected(&self) -> bool {
        self.client_info.connected.load(Ordering::Acquire)
    }
}

/// Errors from hub operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HubError {
    /// Client not found.
    #[error("Client not found")]
    ClientNotFound,
    /// Client is disconnected.
    #[error("Client disconnected")]
    ClientDisconnected,
    /// Permission denied for this operation.
    #[error("Permission denied")]
    PermissionDenied,
    /// Another client has exclusive control.
    #[error("Another client has exclusive control")]
    ExclusiveControlActive,
    /// Command buffer is full.
    #[error("Command buffer full")]
    CommandBufferFull,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ringbuf::HeapRb;

    fn create_test_hub() -> EngineHub {
        let shared_state = Arc::new(SharedEngineState::new());
        let command_rb = HeapRb::<EngineCommand>::new(256);
        let (command_prod, _command_cons) = command_rb.split();
        EngineHub::new(shared_state, command_prod)
    }

    #[test]
    fn test_client_registration() {
        let hub = create_test_hub();

        let handle = hub.register_primary();
        assert_eq!(handle.id, ClientId::PRIMARY);
        assert!(handle.permissions.can_modify_params);
        assert!(handle.permissions.can_modify_topology);

        let handle2 = hub.register_client(ClientType::Observer, ClientPermissions::read_only());
        assert_ne!(handle2.id, ClientId::PRIMARY);
        assert!(!handle2.permissions.can_modify_params);

        assert_eq!(hub.client_count(), 2);
    }

    #[test]
    fn test_client_unregister() {
        let hub = create_test_hub();

        let handle = hub.register_client(ClientType::RemoteGui, ClientPermissions::remote_gui());
        let id = handle.id;

        assert!(handle.is_connected());
        hub.unregister_client(id);

        // Handle should show disconnected
        assert!(!handle.is_connected());
    }

    #[test]
    fn test_exclusive_control() {
        let hub = create_test_hub();

        let primary = hub.register_primary();
        let remote = hub.register_client(ClientType::RemoteGui, ClientPermissions::remote_gui());

        // Primary can get exclusive
        assert!(hub.request_exclusive(primary.id).is_ok());

        // Remote cannot while primary has it
        assert_eq!(
            hub.request_exclusive(remote.id),
            Err(HubError::ExclusiveControlActive)
        );

        // Release and remote can get it
        hub.release_exclusive(primary.id);
        assert!(hub.request_exclusive(remote.id).is_ok());
    }

    #[test]
    fn test_focused_modules() {
        let hub = create_test_hub();
        let handle = hub.register_primary();

        let mod1 = ModuleId::new(synth_core::ModuleType::Oscillator, 1);
        let mod2 = ModuleId::new(synth_core::ModuleType::Filter, 1);

        handle.add_focused_module(mod1);
        handle.add_focused_module(mod2);

        let focused = handle.focused_modules();
        assert_eq!(focused.len(), 2);
        assert!(focused.contains(&mod1));
        assert!(focused.contains(&mod2));

        handle.remove_focused_module(mod1);
        let focused = handle.focused_modules();
        assert_eq!(focused.len(), 1);
        assert!(!focused.contains(&mod1));
    }
}
