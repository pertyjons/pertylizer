# GUI-Engine Kommunikation och Visuell Feedback

## Del 1: Analys av Nuvarande Arkitektur

### Nuvarande Kommunikationskanaler

```
┌────────────────┐                           ┌────────────────┐
│    GUI Thread  │                           │  Audio Thread  │
├────────────────┤                           ├────────────────┤
│                │  EngineCommand            │                │
│  EngineHandle  │ ─────────────────────────>│  SynthEngine   │
│                │  (HeapRb 16384)           │                │
│                │                           │                │
│                │  EngineEvent              │                │
│                │ <─────────────────────────│                │
│                │  (HeapRb 256)             │                │
│                │                           │                │
│                │  DroppedModule            │                │
│                │ <─────────────────────────│                │
│                │  (HeapRb 256)             │                │
│                │                           │                │
│  EngineState   │ ─ ─ ─ Arc<> ─ ─ ─ ─ ─ ─ ─│                │
│  (atomics)     │                           │                │
│                │                           │                │
│ VisualizationBuffer ─ Arc<> ─ ─ ─ ─ ─ ─ ─>│                │
└────────────────┘                           └────────────────┘
```

### Identifierade Problem

#### 1. **Event Ring Buffer Overflow**
- EVENT_BUFFER_SIZE är bara 256
- Vid snabba parameterändringar eller många röster kan events tappas
- Inget rapporteringssystem för tappade events

#### 2. **Inkonsekvent State**
- GUI har inte tillgång till fullständig modul-state
- Ingen synkronisering av graf-topologi till GUI
- GUI vet inte vilka moduler som är uppkopplade

#### 3. **Ingen Transaktionsgaranti**
- Kommandon kan processas delvis vid buffer-overflow
- Patch-laddning kan hamna i inkonsekvent tillstånd

#### 4. **Begränsad Feedback**
Nuvarande EngineEvent har bara:
- PeakMeter, RmsMeter
- VoiceCount
- ParameterChanged (echo)
- CpuUsage
- BufferUnderrun
- EnvelopeStage
- WaveformData

**Saknas:**
- Graf-topologi (vilka moduler är uppkopplade)
- Modul-state (bypassed, muted, solo)
- Detaljerade processingmetriker per modul
- Connection-status
- Error-rapportering

---

## Del 2: Förbättrad Kommunikationsarkitektur

### 2.1 Ny Event-Struktur med Prioritering

```rust
/// Priority levels for engine events.
/// Higher priority events are processed first and have dedicated buffer space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventPriority {
    /// Critical: Buffer underruns, errors, clipping
    Critical = 0,
    /// High: State changes, connections, topology
    High = 1,
    /// Normal: Parameter echoes, voice activity
    Normal = 2,
    /// Low: Visualization data, meters
    Low = 3,
}

/// Enhanced engine event with metadata.
#[derive(Debug, Clone)]
pub struct TimestampedEvent {
    pub event: EngineEvent,
    pub timestamp: u64,        // Sample position when event occurred
    pub priority: EventPriority,
    pub sequence_id: u64,      // Monotonic sequence for ordering
}
```

### 2.2 Multi-Channel Event System

```rust
/// Separate ring buffers per priority to prevent visualization 
/// data from crowding out critical events.
pub struct PrioritizedEventChannel {
    critical: HeapRb<TimestampedEvent>,  // Size: 64, never dropped
    high: HeapRb<TimestampedEvent>,      // Size: 256
    normal: HeapRb<TimestampedEvent>,    // Size: 512
    low: HeapRb<TimestampedEvent>,       // Size: 2048
    
    sequence_counter: AtomicU64,
    dropped_counts: [AtomicU32; 4],  // Track drops per priority
}

impl PrioritizedEventChannel {
    /// Send event - critical events block, others may drop.
    pub fn send(&self, event: EngineEvent, priority: EventPriority) {
        let seq = self.sequence_counter.fetch_add(1, Ordering::Relaxed);
        let timestamped = TimestampedEvent {
            event,
            timestamp: current_sample_position(),
            priority,
            sequence_id: seq,
        };
        
        let buffer = match priority {
            EventPriority::Critical => &self.critical,
            EventPriority::High => &self.high,
            EventPriority::Normal => &self.normal,
            EventPriority::Low => &self.low,
        };
        
        if buffer.try_push(timestamped).is_err() {
            self.dropped_counts[priority as usize].fetch_add(1, Ordering::Relaxed);
            
            // For critical events, try to make room by dropping oldest
            if priority == EventPriority::Critical {
                // Force push by dropping oldest
                // (ring buffer with overwrite mode)
            }
        }
    }
    
    /// Get dropped event counts (for diagnostics).
    pub fn get_dropped_counts(&self) -> [u32; 4] {
        [
            self.dropped_counts[0].swap(0, Ordering::Relaxed),
            self.dropped_counts[1].swap(0, Ordering::Relaxed),
            self.dropped_counts[2].swap(0, Ordering::Relaxed),
            self.dropped_counts[3].swap(0, Ordering::Relaxed),
        ]
    }
}
```

### 2.3 Utökade Events

```rust
/// Extended engine events for complete GUI synchronization.
pub enum EngineEvent {
    // === Existing ===
    PeakMeter { left: f32, right: f32 },
    RmsMeter { left: f32, right: f32 },
    VoiceCount(u32),
    ParameterChanged { module: ModuleId, param: TypedParam, value: TypedValue },
    CpuUsage(f32),
    BufferUnderrun,
    EnvelopeStage { module: ModuleId, stage: u8 },
    WaveformData { data: Vec<f32> },
    
    // === NEW: Topology Events ===
    /// Module added to graph
    ModuleAdded {
        id: ModuleId,
        module_type: ModuleType,
        descriptor: ModuleDescriptorSnapshot,
    },
    
    /// Module removed from graph
    ModuleRemoved {
        id: ModuleId,
    },
    
    /// Connection established
    ConnectionAdded {
        from_module: ModuleId,
        from_port: String,
        to_module: ModuleId,
        to_port: String,
    },
    
    /// Connection removed
    ConnectionRemoved {
        from_module: ModuleId,
        from_port: String,
        to_module: ModuleId,
        to_port: String,
    },
    
    /// Processing order updated (after topology change)
    ProcessingOrderChanged {
        order: Vec<ModuleId>,
    },
    
    // === NEW: Module State Events ===
    /// Module connectivity status changed
    ModuleConnectivityChanged {
        id: ModuleId,
        status: ModuleConnectivityStatus,
    },
    
    /// Module bypass state changed
    ModuleBypassChanged {
        id: ModuleId,
        bypassed: bool,
    },
    
    /// Module error occurred
    ModuleError {
        id: ModuleId,
        error: ModuleErrorKind,
    },
    
    // === NEW: Processing Metrics ===
    /// Per-module CPU usage
    ModuleCpuUsage {
        id: ModuleId,
        usage_percent: f32,
        peak_usage: f32,
    },
    
    /// Per-module output levels (for mini-meters on each module)
    ModuleOutputLevel {
        id: ModuleId,
        port: String,
        level: f32,
    },
    
    // === NEW: Diagnostic Events ===
    /// Events were dropped due to buffer overflow
    EventsDropped {
        priority: EventPriority,
        count: u32,
    },
    
    /// Processing latency warning
    LatencyWarning {
        actual_ms: f32,
        expected_ms: f32,
    },
    
    /// Voice stolen (useful for UI feedback)
    VoiceStolen {
        note: u8,
        reason: VoiceStealReason,
    },
}

/// Snapshot of module descriptor for UI (serializable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleDescriptorSnapshot {
    pub id: String,
    pub name: String,
    pub category: ModuleCategory,
    pub parameters: Vec<ParameterSnapshot>,
    pub ports: Vec<PortSnapshot>,
}

/// Module connectivity status for UI visualization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleConnectivityStatus {
    /// Module has no connections (isolated)
    Disconnected,
    /// Module has some connections but is not in signal path
    PartiallyConnected,
    /// Module is fully connected and contributing to output
    Connected,
    /// Module is connected but bypassed
    Bypassed,
}

/// Reasons for voice stealing (for UI feedback).
#[derive(Debug, Clone, Copy)]
pub enum VoiceStealReason {
    MaxPolyphonyReached,
    SameNoteRetrigger,
    PriorityBased,
}

/// Module error kinds.
#[derive(Debug, Clone)]
pub enum ModuleErrorKind {
    ParameterOutOfRange { param: TypedParam, value: f32 },
    ProcessingOverload,
    InvalidConnection,
    InternalError(String),
}
```

---

## Del 3: Graf-Topologi Synkronisering

### 3.1 Shared Graph State

För att GUI:t ska kunna visualisera modulers uppkopplingsstatus behöver vi delat state:

```rust
use dashmap::DashMap;
use parking_lot::RwLock;

/// Thread-safe snapshot of graph topology.
/// Updated by audio thread, read by GUI thread.
pub struct SharedGraphState {
    /// All modules in the graph
    modules: DashMap<ModuleId, ModuleStateSnapshot>,
    
    /// All connections (source -> destinations)
    connections: RwLock<Vec<ConnectionSnapshot>>,
    
    /// Processing order
    processing_order: RwLock<Vec<ModuleId>>,
    
    /// Which modules are "live" (connected to output)
    live_modules: RwLock<HashSet<ModuleId>>,
    
    /// Version counter - incremented on any topology change
    version: AtomicU64,
}

#[derive(Debug, Clone)]
pub struct ModuleStateSnapshot {
    pub id: ModuleId,
    pub module_type: ModuleType,
    pub name: String,
    pub bypassed: bool,
    pub muted: bool,
    pub solo: bool,
    pub connectivity: ModuleConnectivityStatus,
    
    /// Current parameter values (for UI display)
    pub parameters: HashMap<TypedParam, TypedValue>,
    
    /// Port connection counts
    pub input_connection_counts: HashMap<String, usize>,
    pub output_connection_counts: HashMap<String, usize>,
    
    /// Processing metrics
    pub cpu_usage: f32,
    pub output_levels: HashMap<String, f32>,
}

#[derive(Debug, Clone)]
pub struct ConnectionSnapshot {
    pub from_module: ModuleId,
    pub from_port: String,
    pub to_module: ModuleId,
    pub to_port: String,
    pub signal_level: f32,  // For animated cables
}

impl SharedGraphState {
    /// Check if version changed (cheap operation for GUI polling).
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }
    
    /// Get snapshot of a specific module.
    pub fn get_module(&self, id: ModuleId) -> Option<ModuleStateSnapshot> {
        self.modules.get(&id).map(|r| r.clone())
    }
    
    /// Get all modules with their states.
    pub fn get_all_modules(&self) -> Vec<ModuleStateSnapshot> {
        self.modules.iter().map(|r| r.clone()).collect()
    }
    
    /// Check if a module is connected to output.
    pub fn is_live(&self, id: ModuleId) -> bool {
        self.live_modules.read().contains(&id)
    }
    
    /// Get connectivity status for a module.
    pub fn connectivity(&self, id: ModuleId) -> ModuleConnectivityStatus {
        self.modules.get(&id)
            .map(|m| m.connectivity)
            .unwrap_or(ModuleConnectivityStatus::Disconnected)
    }
}
```

### 3.2 Live Module Detection

```rust
impl ModuleGraph {
    /// Calculate which modules are "live" (contribute to output).
    /// Uses reverse traversal from output modules.
    pub fn calculate_live_modules(&self) -> HashSet<ModuleId> {
        let mut live = HashSet::new();
        let mut to_visit = VecDeque::new();
        
        // Start from output modules
        for (&id, node) in &self.nodes {
            if node.descriptor.category == ModuleCategory::Output {
                to_visit.push_back(id);
            }
        }
        
        // Also include sink modules (no outgoing connections but has incoming)
        for &id in &self.processing_order {
            if self.is_sink(id) {
                let has_incoming = self.connections.iter()
                    .any(|c| c.to_module == id);
                if has_incoming {
                    to_visit.push_back(id);
                }
            }
        }
        
        // Traverse backwards through connections
        while let Some(id) = to_visit.pop_front() {
            if live.insert(id) {
                // Find all modules that connect TO this module
                for conn in &self.connections {
                    if conn.to_module == id && !live.contains(&conn.from_module) {
                        to_visit.push_back(conn.from_module);
                    }
                }
            }
        }
        
        live
    }
    
    /// Determine connectivity status for a module.
    pub fn module_connectivity(&self, id: ModuleId, live_modules: &HashSet<ModuleId>) 
        -> ModuleConnectivityStatus 
    {
        let has_incoming = self.connections.iter().any(|c| c.to_module == id);
        let has_outgoing = self.connections.iter().any(|c| c.from_module == id);
        let is_live = live_modules.contains(&id);
        
        // Check if module is bypassed
        if let Some(node) = self.nodes.get(&id) {
            if node.module.is_bypassed() {
                return ModuleConnectivityStatus::Bypassed;
            }
        }
        
        match (has_incoming || has_outgoing, is_live) {
            (false, _) => ModuleConnectivityStatus::Disconnected,
            (true, false) => ModuleConnectivityStatus::PartiallyConnected,
            (true, true) => ModuleConnectivityStatus::Connected,
        }
    }
    
    /// Update shared graph state after topology change.
    pub fn sync_to_shared_state(&self, shared: &SharedGraphState) {
        let live_modules = self.calculate_live_modules();
        
        // Update each module's state
        for (&id, node) in &self.nodes {
            let connectivity = self.module_connectivity(id, &live_modules);
            
            // Count connections per port
            let mut input_counts = HashMap::new();
            let mut output_counts = HashMap::new();
            
            for conn in &self.connections {
                if conn.to_module == id {
                    *input_counts.entry(conn.to_port.clone()).or_insert(0) += 1;
                }
                if conn.from_module == id {
                    *output_counts.entry(conn.from_port.clone()).or_insert(0) += 1;
                }
            }
            
            let snapshot = ModuleStateSnapshot {
                id,
                module_type: id.module_type,
                name: node.descriptor.name.clone(),
                bypassed: node.module.is_bypassed(),
                muted: false,  // TODO: implement mute
                solo: false,   // TODO: implement solo
                connectivity,
                parameters: node.module.get_all_params(),
                input_connection_counts: input_counts,
                output_connection_counts: output_counts,
                cpu_usage: 0.0,  // Updated separately
                output_levels: HashMap::new(),
            };
            
            shared.modules.insert(id, snapshot);
        }
        
        // Update connections
        {
            let mut conns = shared.connections.write();
            *conns = self.connections.iter().map(|c| ConnectionSnapshot {
                from_module: c.from_module,
                from_port: c.from_port.clone(),
                to_module: c.to_module,
                to_port: c.to_port.clone(),
                signal_level: 0.0,  // Updated during processing
            }).collect();
        }
        
        // Update live modules
        {
            let mut live = shared.live_modules.write();
            *live = live_modules;
        }
        
        // Update processing order
        {
            let mut order = shared.processing_order.write();
            *order = self.processing_order.clone();
        }
        
        // Increment version
        shared.version.fetch_add(1, Ordering::Release);
    }
}
```

---

## Del 4: GUI Visualiseringskomponenter

### 4.1 Module Widget State

```rust
/// Visual state for a module in the GUI.
pub struct ModuleVisualState {
    /// Connectivity status determines visual appearance
    pub connectivity: ModuleConnectivityStatus,
    
    /// Animation states
    pub opacity_target: f32,
    pub opacity_current: f32,
    pub glow_intensity: f32,  // For "live" indication
    pub pulse_phase: f32,     // For activity animation
    
    /// Port states
    pub port_states: HashMap<String, PortVisualState>,
    
    /// Error state
    pub error: Option<ModuleErrorKind>,
    pub error_flash_time: f32,
}

#[derive(Debug, Clone)]
pub struct PortVisualState {
    pub connected: bool,
    pub signal_level: f32,
    pub is_active: bool,  // Signal flowing
}

impl ModuleVisualState {
    /// Update visual state from graph state.
    pub fn update_from_snapshot(&mut self, snapshot: &ModuleStateSnapshot, dt: f32) {
        // Smooth opacity transition
        self.opacity_target = match snapshot.connectivity {
            ModuleConnectivityStatus::Disconnected => 0.4,
            ModuleConnectivityStatus::PartiallyConnected => 0.7,
            ModuleConnectivityStatus::Connected => 1.0,
            ModuleConnectivityStatus::Bypassed => 0.5,
        };
        
        // Lerp current towards target
        let lerp_speed = 5.0;
        self.opacity_current += (self.opacity_target - self.opacity_current) * lerp_speed * dt;
        
        // Glow for live modules
        if snapshot.connectivity == ModuleConnectivityStatus::Connected {
            self.glow_intensity = 0.3 + snapshot.cpu_usage * 0.5;
        } else {
            self.glow_intensity = 0.0;
        }
        
        // Pulse animation based on output level
        if let Some(&level) = snapshot.output_levels.get("out") {
            if level > 0.01 {
                self.pulse_phase = (self.pulse_phase + dt * 10.0) % (2.0 * PI);
            }
        }
        
        // Update port states
        for (port_name, &count) in &snapshot.input_connection_counts {
            self.port_states.entry(port_name.clone())
                .or_insert_with(Default::default)
                .connected = count > 0;
        }
        for (port_name, &count) in &snapshot.output_connection_counts {
            self.port_states.entry(port_name.clone())
                .or_insert_with(Default::default)
                .connected = count > 0;
        }
    }
    
    /// Get the visual style for rendering.
    pub fn get_style(&self) -> ModuleStyle {
        ModuleStyle {
            background_opacity: self.opacity_current,
            border_color: match self.connectivity {
                ModuleConnectivityStatus::Disconnected => Color::rgba(0.5, 0.5, 0.5, 0.5),
                ModuleConnectivityStatus::PartiallyConnected => Color::rgba(0.8, 0.6, 0.2, 0.8),
                ModuleConnectivityStatus::Connected => Color::rgba(0.2, 0.8, 0.4, 1.0),
                ModuleConnectivityStatus::Bypassed => Color::rgba(0.6, 0.6, 0.6, 0.7),
            },
            glow_color: Color::rgba(0.3, 0.7, 1.0, self.glow_intensity),
            error_overlay: self.error.is_some(),
            pulse_intensity: (self.pulse_phase.sin() * 0.5 + 0.5) * 0.2,
        }
    }
}
```

### 4.2 Connection Cable Visualization

```rust
/// Visual state for a connection cable.
pub struct CableVisualState {
    pub from_pos: Point,
    pub to_pos: Point,
    pub signal_level: f32,
    pub flow_phase: f32,  // Animation for signal flow direction
    pub color: Color,
}

impl CableVisualState {
    /// Update cable animation.
    pub fn update(&mut self, signal_level: f32, dt: f32) {
        // Smooth signal level
        self.signal_level = self.signal_level * 0.9 + signal_level * 0.1;
        
        // Animate flow based on signal presence
        if self.signal_level > 0.001 {
            self.flow_phase = (self.flow_phase + dt * self.signal_level * 5.0) % 1.0;
        }
        
        // Color based on signal level
        let intensity = self.signal_level.sqrt();
        self.color = Color::rgba(
            0.3 + intensity * 0.5,  // More red at high levels
            0.7 - intensity * 0.3,
            0.9,
            0.6 + intensity * 0.4,
        );
    }
    
    /// Render cable with animated flow.
    pub fn render(&self, ctx: &mut RenderContext) {
        // Bezier curve between points
        let control1 = Point::new(
            self.from_pos.x + 50.0,
            self.from_pos.y,
        );
        let control2 = Point::new(
            self.to_pos.x - 50.0,
            self.to_pos.y,
        );
        
        // Draw cable shadow
        ctx.stroke_bezier(
            self.from_pos, control1, control2, self.to_pos,
            &Stroke::new(4.0, Color::rgba(0.0, 0.0, 0.0, 0.3)),
        );
        
        // Draw main cable
        ctx.stroke_bezier(
            self.from_pos, control1, control2, self.to_pos,
            &Stroke::new(2.0, self.color),
        );
        
        // Draw flow animation (moving dots along cable)
        if self.signal_level > 0.001 {
            let num_dots = 3;
            for i in 0..num_dots {
                let t = (self.flow_phase + i as f32 / num_dots as f32) % 1.0;
                let pos = bezier_point(self.from_pos, control1, control2, self.to_pos, t);
                
                ctx.fill_circle(
                    pos,
                    2.0 + self.signal_level * 2.0,
                    Color::rgba(1.0, 1.0, 1.0, 0.8 * self.signal_level),
                );
            }
        }
    }
}
```

### 4.3 Mini Level Meters per Modul

```rust
/// Tiny level meter for module output visualization.
pub struct MiniMeter {
    pub level: f32,
    pub peak: f32,
    pub peak_hold_time: f32,
    pub width: f32,
    pub height: f32,
}

impl MiniMeter {
    pub fn update(&mut self, new_level: f32, dt: f32) {
        // Smooth level
        self.level = self.level * 0.8 + new_level * 0.2;
        
        // Peak hold
        if new_level > self.peak {
            self.peak = new_level;
            self.peak_hold_time = 1.0;  // Hold for 1 second
        } else {
            self.peak_hold_time -= dt;
            if self.peak_hold_time <= 0.0 {
                self.peak *= 0.99;  // Slow decay
            }
        }
    }
    
    pub fn render(&self, ctx: &mut RenderContext, pos: Point) {
        // Background
        ctx.fill_rect(
            Rect::new(pos.x, pos.y, self.width, self.height),
            Color::rgba(0.1, 0.1, 0.1, 0.8),
        );
        
        // Level bar (green to yellow to red)
        let level_height = self.height * self.level.min(1.0);
        let color = if self.level > 0.9 {
            Color::rgb(1.0, 0.2, 0.2)  // Red (clipping)
        } else if self.level > 0.7 {
            Color::rgb(1.0, 0.8, 0.2)  // Yellow (hot)
        } else {
            Color::rgb(0.2, 0.8, 0.4)  // Green (normal)
        };
        
        ctx.fill_rect(
            Rect::new(pos.x, pos.y + self.height - level_height, self.width, level_height),
            color,
        );
        
        // Peak indicator
        if self.peak > 0.01 {
            let peak_y = pos.y + self.height * (1.0 - self.peak.min(1.0));
            ctx.fill_rect(
                Rect::new(pos.x, peak_y, self.width, 2.0),
                Color::rgb(1.0, 1.0, 1.0),
            );
        }
    }
}
```

---

## Del 5: Transaktionell Command System

### 5.1 Command Batching

```rust
/// Batch multiple commands as a single atomic operation.
/// Useful for patch loading, undo/redo, etc.
pub struct CommandBatch {
    commands: Vec<EngineCommand>,
    transaction_id: u64,
}

impl CommandBatch {
    pub fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        Self {
            commands: Vec::new(),
            transaction_id: COUNTER.fetch_add(1, Ordering::Relaxed),
        }
    }
    
    pub fn add(&mut self, cmd: EngineCommand) {
        self.commands.push(cmd);
    }
    
    /// Calculate required buffer space.
    pub fn required_space(&self) -> usize {
        self.commands.len() + 2  // +2 for start/end markers
    }
}

/// Wrapper commands for transactional semantics.
pub enum TransactionalCommand {
    /// Single command (as before)
    Single(EngineCommand),
    
    /// Start of a transaction batch
    TransactionStart {
        id: u64,
        expected_count: usize,
    },
    
    /// End of a transaction batch
    TransactionEnd {
        id: u64,
    },
    
    /// Rollback current transaction
    TransactionRollback {
        id: u64,
    },
}

impl EngineHandle {
    /// Send a batch of commands atomically.
    /// Returns false if there isn't enough buffer space.
    pub fn send_batch(&mut self, batch: CommandBatch) -> bool {
        let required = batch.required_space();
        
        // Check if we have enough space
        let available = self.command_producer.vacant_len();
        if available < required {
            // Wait for space (with timeout)
            let start = Instant::now();
            while self.command_producer.vacant_len() < required {
                if start.elapsed() > Duration::from_secs(5) {
                    return false;  // Timeout
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        
        // Send transaction start
        let _ = self.command_producer.try_push(TransactionalCommand::TransactionStart {
            id: batch.transaction_id,
            expected_count: batch.commands.len(),
        });
        
        // Send all commands
        for cmd in batch.commands {
            let _ = self.command_producer.try_push(TransactionalCommand::Single(cmd));
        }
        
        // Send transaction end
        let _ = self.command_producer.try_push(TransactionalCommand::TransactionEnd {
            id: batch.transaction_id,
        });
        
        true
    }
}
```

### 5.2 Engine Transaction Processing

```rust
impl SynthEngine {
    fn process_commands(&mut self) {
        let mut current_transaction: Option<TransactionState> = None;
        
        while let Some(cmd) = self.command_consumer.try_pop() {
            match cmd {
                TransactionalCommand::TransactionStart { id, expected_count } => {
                    current_transaction = Some(TransactionState {
                        id,
                        expected_count,
                        received_count: 0,
                        commands: Vec::with_capacity(expected_count),
                    });
                }
                
                TransactionalCommand::Single(cmd) => {
                    if let Some(ref mut tx) = current_transaction {
                        tx.commands.push(cmd);
                        tx.received_count += 1;
                    } else {
                        // Direct execution
                        self.execute_command(cmd);
                    }
                }
                
                TransactionalCommand::TransactionEnd { id } => {
                    if let Some(tx) = current_transaction.take() {
                        if tx.id == id && tx.received_count == tx.expected_count {
                            // Execute all commands atomically
                            for cmd in tx.commands {
                                self.execute_command(cmd);
                            }
                            // Sync graph state after batch
                            self.sync_graph_state();
                        } else {
                            // Transaction incomplete - log error
                            eprintln!("Transaction {} incomplete: expected {}, got {}",
                                id, tx.expected_count, tx.received_count);
                        }
                    }
                }
                
                TransactionalCommand::TransactionRollback { id } => {
                    if let Some(tx) = current_transaction.take() {
                        if tx.id == id {
                            // Discard all buffered commands
                            eprintln!("Transaction {} rolled back", id);
                        }
                    }
                }
            }
        }
    }
}
```

---

## Del 6: Per-Module Processing Metrics

### 6.1 CPU Usage Tracking

```rust
/// Tracks CPU usage per module for visualization.
pub struct ModuleCpuTracker {
    modules: HashMap<ModuleId, ModuleTiming>,
    frame_budget_ns: u64,
}

struct ModuleTiming {
    last_duration_ns: u64,
    avg_duration_ns: f32,
    peak_duration_ns: u64,
    peak_decay_counter: u32,
}

impl ModuleCpuTracker {
    pub fn new(sample_rate: f32, buffer_size: usize) -> Self {
        // Calculate frame budget in nanoseconds
        let frame_duration_s = buffer_size as f64 / sample_rate as f64;
        let frame_budget_ns = (frame_duration_s * 1_000_000_000.0) as u64;
        
        Self {
            modules: HashMap::new(),
            frame_budget_ns,
        }
    }
    
    /// Start timing a module.
    pub fn start(&self) -> Instant {
        Instant::now()
    }
    
    /// End timing and record.
    pub fn end(&mut self, module_id: ModuleId, start: Instant) {
        let duration_ns = start.elapsed().as_nanos() as u64;
        
        let timing = self.modules.entry(module_id).or_insert(ModuleTiming {
            last_duration_ns: 0,
            avg_duration_ns: 0.0,
            peak_duration_ns: 0,
            peak_decay_counter: 0,
        });
        
        timing.last_duration_ns = duration_ns;
        
        // Exponential moving average
        timing.avg_duration_ns = timing.avg_duration_ns * 0.95 + duration_ns as f32 * 0.05;
        
        // Peak tracking with decay
        if duration_ns > timing.peak_duration_ns {
            timing.peak_duration_ns = duration_ns;
            timing.peak_decay_counter = 100;  // Hold for 100 frames
        } else {
            if timing.peak_decay_counter > 0 {
                timing.peak_decay_counter -= 1;
            } else {
                timing.peak_duration_ns = (timing.peak_duration_ns as f32 * 0.99) as u64;
            }
        }
    }
    
    /// Get CPU usage for a module as percentage of frame budget.
    pub fn get_usage(&self, module_id: ModuleId) -> Option<(f32, f32)> {
        self.modules.get(&module_id).map(|t| {
            let avg = t.avg_duration_ns as f64 / self.frame_budget_ns as f64 * 100.0;
            let peak = t.peak_duration_ns as f64 / self.frame_budget_ns as f64 * 100.0;
            (avg as f32, peak as f32)
        })
    }
    
    /// Get all module usages.
    pub fn get_all_usages(&self) -> Vec<(ModuleId, f32, f32)> {
        self.modules.iter().map(|(&id, t)| {
            let avg = t.avg_duration_ns as f64 / self.frame_budget_ns as f64 * 100.0;
            let peak = t.peak_duration_ns as f64 / self.frame_budget_ns as f64 * 100.0;
            (id, avg as f32, peak as f32)
        }).collect()
    }
}
```

### 6.2 Integration i Graph Processing

```rust
impl ModuleGraph {
    /// Process with CPU tracking.
    pub fn process_with_metrics(
        &mut self, 
        output: &mut AudioBuffer, 
        context: &ProcessContext,
        cpu_tracker: &mut ModuleCpuTracker,
        level_tracker: &mut ModuleLevelTracker,
    ) {
        self.resize_buffers(context.samples);
        
        if self.order_dirty {
            self.calculate_processing_order();
        }
        
        for i in 0..self.processing_order.len() {
            let module_id = self.processing_order[i];
            
            // Time module processing
            let start = cpu_tracker.start();
            self.process_module(module_id, context);
            cpu_tracker.end(module_id, start);
            
            // Track output levels
            if let Some(node) = self.nodes.get(&module_id) {
                for (port_name, buffer) in &node.outputs {
                    let level = buffer.as_slice().iter()
                        .map(|s| s.abs())
                        .fold(0.0f32, f32::max);
                    level_tracker.update(module_id, port_name, level);
                }
            }
        }
        
        // Copy output (existing logic)...
    }
}
```

---

## Del 7: Implementeringsplan

### Prioritet 1 (Kritisk infrastruktur)

1. **PrioritizedEventChannel** - Förhindra event-förlust
2. **SharedGraphState** - Möjliggör GUI-synkronisering
3. **ModuleConnectivityStatus** - Grundläggande visuell feedback

### Prioritet 2 (GUI-förbättringar)

4. **ModuleVisualState** - Animerad modulvisning
5. **CableVisualState** - Animerade kablar
6. **MiniMeter** - Per-modul nivåvisning

### Prioritet 3 (Avancerad feedback)

7. **CommandBatch/Transactions** - Atomisk patch-laddning
8. **ModuleCpuTracker** - CPU per modul
9. **Detaljerade EngineEvents** - Fullständig diagnostik

### Prioritet 4 (Polish)

10. **Error visualization** - Visuell felindikation
11. **Voice stealing feedback** - Användaren ser när röster stjäls
12. **Latency warnings** - Varningar vid överbelastning

---

## Del 8: Multi-GUI Arkitektur

### 8.1 Problem med Nuvarande Design

Nuvarande arkitektur har en **1:1 relation** mellan `SynthEngine` och `EngineHandle`:

```rust
// Nuvarande - fungerar INTE för multi-GUI
pub fn new() -> (Self, EngineHandle) {
    // Ring buffers splittas - endast EN producer/consumer per kanal
    let (command_producer, command_consumer) = command_rb.split();
    let (event_producer, event_consumer) = event_rb.split();
}
```

**Fundamentala problem:**
1. Ring buffer split ger exakt en producer och en consumer
2. `EngineHandle` äger sin `command_producer` - kan inte klonas
3. Events skickas till EN consumer - andra GUI:n ser ingenting
4. `VisualizationBuffer` använder `Mutex` som kan blockera

### 8.2 Multi-GUI Arkitektur: Hub-and-Spoke

```
                    ┌─────────────────┐
     ┌──────────────│  Audio Thread   │──────────────┐
     │              │  SynthEngine    │              │
     │              └────────┬────────┘              │
     │                       │                       │
     │              ┌────────▼────────┐              │
     │              │   EngineHub     │              │
     │              │  (Main Thread)  │              │
     │              └────────┬────────┘              │
     │                       │                       │
     │         ┌─────────────┼─────────────┐         │
     │         │             │             │         │
     ▼         ▼             ▼             ▼         ▼
┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐
│ GUI #1  │ │ GUI #2  │ │ GUI #3  │ │  OSC    │ │  MIDI   │
│ (Local) │ │ (Local) │ │(Network)│ │ Client  │ │ Control │
└─────────┘ └─────────┘ └─────────┘ └─────────┘ └─────────┘
```

### 8.3 EngineHub - Central Distributor

```rust
use std::sync::Arc;
use parking_lot::RwLock;
use tokio::sync::broadcast;

/// Central hub that manages multiple GUI connections.
/// Runs on the main thread and bridges to the audio engine.
pub struct EngineHub {
    // === Connection to audio engine ===
    engine_handle: EngineHandle,
    
    // === Shared state (read by all clients) ===
    shared_state: Arc<SharedEngineState>,
    
    // === Event broadcasting ===
    event_broadcaster: broadcast::Sender<Arc<EngineEvent>>,
    
    // === Client management ===
    clients: RwLock<Vec<ClientInfo>>,
    next_client_id: AtomicU64,
    
    // === Command aggregation ===
    command_receiver: mpsc::Receiver<ClientCommand>,
    command_sender: mpsc::Sender<ClientCommand>,  // Cloned to clients
}

/// Information about a connected client.
struct ClientInfo {
    id: u64,
    name: String,
    client_type: ClientType,
    connected_at: Instant,
    last_activity: Instant,
    permissions: ClientPermissions,
}

#[derive(Debug, Clone, Copy)]
pub enum ClientType {
    LocalGui,
    NetworkGui,
    OscController,
    MidiController,
    Automation,  // For DAW control
}

/// What a client is allowed to do.
#[derive(Debug, Clone)]
pub struct ClientPermissions {
    pub can_modify_parameters: bool,
    pub can_modify_topology: bool,
    pub can_load_patches: bool,
    pub can_control_transport: bool,
    pub read_only: bool,
}

impl Default for ClientPermissions {
    fn default() -> Self {
        Self {
            can_modify_parameters: true,
            can_modify_topology: true,
            can_load_patches: true,
            can_control_transport: true,
            read_only: false,
        }
    }
}

/// Command from a client, tagged with client ID for conflict resolution.
#[derive(Debug)]
struct ClientCommand {
    client_id: u64,
    command: EngineCommand,
    timestamp: Instant,
    transaction_id: Option<u64>,
}

impl EngineHub {
    pub fn new(engine_handle: EngineHandle) -> Self {
        let (tx, rx) = mpsc::channel(4096);
        let (event_tx, _) = broadcast::channel(1024);
        
        Self {
            shared_state: Arc::new(SharedEngineState::new()),
            engine_handle,
            event_broadcaster: event_tx,
            clients: RwLock::new(Vec::new()),
            next_client_id: AtomicU64::new(1),
            command_receiver: rx,
            command_sender: tx,
        }
    }
    
    /// Create a new client handle.
    pub fn connect(&self, name: String, client_type: ClientType) -> ClientHandle {
        let id = self.next_client_id.fetch_add(1, Ordering::Relaxed);
        
        let info = ClientInfo {
            id,
            name: name.clone(),
            client_type,
            connected_at: Instant::now(),
            last_activity: Instant::now(),
            permissions: ClientPermissions::default(),
        };
        
        self.clients.write().push(info);
        
        ClientHandle {
            id,
            shared_state: Arc::clone(&self.shared_state),
            command_sender: self.command_sender.clone(),
            event_receiver: self.event_broadcaster.subscribe(),
        }
    }
    
    /// Main loop - run on dedicated thread or async runtime.
    pub async fn run(&mut self) {
        loop {
            tokio::select! {
                // Process incoming commands from clients
                Some(cmd) = self.command_receiver.recv() => {
                    self.handle_client_command(cmd).await;
                }
                
                // Poll events from engine
                _ = tokio::time::sleep(Duration::from_micros(500)) => {
                    self.poll_engine_events();
                    self.update_shared_state();
                }
            }
        }
    }
    
    fn handle_client_command(&mut self, cmd: ClientCommand) {
        // Check permissions
        let clients = self.clients.read();
        let client = clients.iter().find(|c| c.id == cmd.client_id);
        
        if let Some(client) = client {
            if !self.check_permission(client, &cmd.command) {
                // Send permission denied event back
                let _ = self.event_broadcaster.send(Arc::new(
                    EngineEvent::PermissionDenied {
                        client_id: cmd.client_id,
                        command_type: cmd.command.type_name(),
                    }
                ));
                return;
            }
        }
        
        // Handle conflict resolution for simultaneous edits
        if self.detect_conflict(&cmd) {
            // Last-write-wins or merge strategy
            self.resolve_conflict(&cmd);
        }
        
        // Forward to engine
        self.engine_handle.send(cmd.command);
    }
    
    fn poll_engine_events(&mut self) {
        while let Some(event) = self.engine_handle.poll_event() {
            // Broadcast to all clients
            // broadcast::Sender handles the case where no receivers exist
            let _ = self.event_broadcaster.send(Arc::new(event));
        }
        
        // Also cleanup dropped modules
        self.engine_handle.cleanup_dropped_modules();
    }
    
    fn update_shared_state(&self) {
        // Update atomic values that all clients can read
        let (peak_l, peak_r) = self.engine_handle.peak_meters();
        self.shared_state.meters.peak_left.store(peak_l);
        self.shared_state.meters.peak_right.store(peak_r);
        
        self.shared_state.voice_count.store(self.engine_handle.voice_count());
        self.shared_state.cpu_usage.store(self.engine_handle.cpu_usage());
    }
}
```

### 8.4 ClientHandle - Per-GUI Interface

```rust
/// Handle for a single GUI/client to communicate with the engine.
/// Multiple ClientHandles can exist simultaneously.
#[derive(Clone)]
pub struct ClientHandle {
    id: u64,
    shared_state: Arc<SharedEngineState>,
    command_sender: mpsc::Sender<ClientCommand>,
    event_receiver: broadcast::Receiver<Arc<EngineEvent>>,
}

impl ClientHandle {
    /// Send a command to the engine.
    pub async fn send(&self, command: EngineCommand) -> Result<(), SendError> {
        self.command_sender.send(ClientCommand {
            client_id: self.id,
            command,
            timestamp: Instant::now(),
            transaction_id: None,
        }).await.map_err(|_| SendError::Disconnected)
    }
    
    /// Send command synchronously (for non-async contexts).
    pub fn send_blocking(&self, command: EngineCommand) -> Result<(), SendError> {
        self.command_sender.blocking_send(ClientCommand {
            client_id: self.id,
            command,
            timestamp: Instant::now(),
            transaction_id: None,
        }).map_err(|_| SendError::Disconnected)
    }
    
    /// Receive next event (async).
    pub async fn recv_event(&mut self) -> Result<Arc<EngineEvent>, RecvError> {
        self.event_receiver.recv().await.map_err(|_| RecvError::Closed)
    }
    
    /// Try to receive event without blocking.
    pub fn try_recv_event(&mut self) -> Option<Arc<EngineEvent>> {
        self.event_receiver.try_recv().ok()
    }
    
    // === Direct state access (no event needed) ===
    
    pub fn peak_meters(&self) -> (f32, f32) {
        (
            self.shared_state.meters.peak_left.load(),
            self.shared_state.meters.peak_right.load(),
        )
    }
    
    pub fn voice_count(&self) -> u32 {
        self.shared_state.voice_count.load()
    }
    
    pub fn cpu_usage(&self) -> f32 {
        self.shared_state.cpu_usage.load()
    }
    
    /// Get full graph state snapshot.
    pub fn graph_state(&self) -> &SharedGraphState {
        &self.shared_state.graph
    }
    
    /// Check if a module is connected.
    pub fn is_module_connected(&self, id: ModuleId) -> bool {
        self.shared_state.graph.is_live(id)
    }
}
```

### 8.5 SharedEngineState - Lock-Free Shared Data

```rust
/// State shared between all clients via Arc.
/// Uses atomic operations and lock-free structures for thread safety.
pub struct SharedEngineState {
    // === Meters (atomic, updated frequently) ===
    pub meters: MeterState,
    pub voice_count: AtomicU32,
    pub cpu_usage: AtomicF32,
    
    // === Transport ===
    pub transport: TransportState,
    
    // === Graph topology (RwLock, updated less frequently) ===
    pub graph: SharedGraphState,
    
    // === Parameter values (DashMap for concurrent access) ===
    pub parameters: DashMap<(ModuleId, TypedParam), TypedValue>,
    
    // === Module descriptors (read-only after creation) ===
    pub descriptors: DashMap<ModuleId, ModuleDescriptorSnapshot>,
    
    // === Version for change detection ===
    pub version: AtomicU64,
}

impl SharedEngineState {
    pub fn new() -> Self {
        Self {
            meters: MeterState::default(),
            voice_count: AtomicU32::new(0),
            cpu_usage: AtomicF32::new(0.0),
            transport: TransportState::new(),
            graph: SharedGraphState::new(),
            parameters: DashMap::new(),
            descriptors: DashMap::new(),
            version: AtomicU64::new(0),
        }
    }
    
    /// Get current parameter value (lock-free read).
    pub fn get_param(&self, module: ModuleId, param: TypedParam) -> Option<TypedValue> {
        self.parameters.get(&(module, param)).map(|r| r.clone())
    }
    
    /// Check if state changed since last check.
    pub fn changed_since(&self, last_version: u64) -> bool {
        self.version.load(Ordering::Acquire) > last_version
    }
}
```

### 8.6 Konflikthantering

```rust
/// Strategy for handling conflicting commands from multiple clients.
#[derive(Debug, Clone, Copy)]
pub enum ConflictStrategy {
    /// Last command wins (simple, may cause jumps)
    LastWriteWins,
    
    /// First command wins, reject later ones
    FirstWriteWins,
    
    /// Merge changes if possible (for compatible changes)
    Merge,
    
    /// Lock parameter while being edited
    Locking,
}

/// Tracks parameter locks for Locking strategy.
struct ParameterLockManager {
    locks: DashMap<(ModuleId, TypedParam), ParameterLock>,
}

struct ParameterLock {
    owner_client_id: u64,
    acquired_at: Instant,
    timeout: Duration,
}

impl EngineHub {
    fn detect_conflict(&self, cmd: &ClientCommand) -> bool {
        // Check if another client recently modified the same parameter
        if let EngineCommand::SetVoiceParameter { target, param, .. } = &cmd.command {
            let key = (target.module_id(), param.clone());
            
            if let Some(last_edit) = self.recent_edits.get(&key) {
                if last_edit.client_id != cmd.client_id 
                    && last_edit.timestamp.elapsed() < Duration::from_millis(100) 
                {
                    return true;
                }
            }
        }
        false
    }
    
    fn resolve_conflict(&mut self, cmd: &ClientCommand) {
        match self.conflict_strategy {
            ConflictStrategy::LastWriteWins => {
                // Just proceed - last command overwrites
            }
            
            ConflictStrategy::FirstWriteWins => {
                // Reject this command, notify client
                let _ = self.event_broadcaster.send(Arc::new(
                    EngineEvent::CommandRejected {
                        client_id: cmd.client_id,
                        reason: "Parameter was modified by another client".into(),
                    }
                ));
            }
            
            ConflictStrategy::Locking => {
                // Check if we hold the lock
                if !self.lock_manager.check_lock(cmd.client_id, &cmd.command) {
                    let _ = self.event_broadcaster.send(Arc::new(
                        EngineEvent::CommandRejected {
                            client_id: cmd.client_id,
                            reason: "Parameter is locked by another client".into(),
                        }
                    ));
                }
            }
            
            ConflictStrategy::Merge => {
                // For numeric parameters, could interpolate
                // For topology, queue for later
            }
        }
    }
}
```

### 8.7 Network GUI Support

```rust
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, WebSocketStream};

/// WebSocket server for remote GUI connections.
pub struct NetworkGuiServer {
    hub: Arc<EngineHub>,
    listener: TcpListener,
}

impl NetworkGuiServer {
    pub async fn new(hub: Arc<EngineHub>, addr: &str) -> Result<Self, std::io::Error> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self { hub, listener })
    }
    
    pub async fn run(&self) {
        while let Ok((stream, addr)) = self.listener.accept().await {
            let hub = Arc::clone(&self.hub);
            tokio::spawn(async move {
                if let Err(e) = handle_connection(hub, stream, addr).await {
                    eprintln!("WebSocket error: {}", e);
                }
            });
        }
    }
}

async fn handle_connection(
    hub: Arc<EngineHub>,
    stream: TcpStream,
    addr: std::net::SocketAddr,
) -> Result<(), Box<dyn std::error::Error>> {
    let ws = accept_async(stream).await?;
    let (mut write, mut read) = ws.split();
    
    // Create client handle
    let mut client = hub.connect(
        format!("Network:{}", addr),
        ClientType::NetworkGui,
    );
    
    // Spawn task to forward events to WebSocket
    let event_tx = tokio::spawn(async move {
        while let Ok(event) = client.recv_event().await {
            let json = serde_json::to_string(&*event)?;
            write.send(Message::Text(json)).await?;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    });
    
    // Read commands from WebSocket
    while let Some(msg) = read.next().await {
        match msg? {
            Message::Text(json) => {
                let cmd: EngineCommand = serde_json::from_str(&json)?;
                client.send(cmd).await?;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    
    event_tx.abort();
    Ok(())
}
```

### 8.8 Visualization Buffer för Multi-GUI

```rust
/// Thread-safe visualization buffer that supports multiple readers.
pub struct SharedVisualizationBuffer {
    /// Triple buffer for waveform data (lock-free swap).
    waveform: TripleBuffer<WaveformData>,
    
    /// Atomic levels (always accessible).
    peak_left: AtomicF32,
    peak_right: AtomicF32,
    rms_left: AtomicF32,
    rms_right: AtomicF32,
    
    /// Subscriber count for backpressure.
    subscriber_count: AtomicU32,
}

/// Triple buffer for lock-free producer-consumer with multiple readers.
struct TripleBuffer<T> {
    buffers: [RwLock<T>; 3],
    write_index: AtomicUsize,
    read_index: AtomicUsize,
}

impl<T: Clone + Default> TripleBuffer<T> {
    fn new() -> Self {
        Self {
            buffers: [
                RwLock::new(T::default()),
                RwLock::new(T::default()),
                RwLock::new(T::default()),
            ],
            write_index: AtomicUsize::new(0),
            read_index: AtomicUsize::new(1),
        }
    }
    
    /// Write new data (audio thread).
    fn write(&self, data: T) {
        let idx = self.write_index.load(Ordering::Relaxed);
        
        // Find free buffer (not being read)
        let next = (idx + 1) % 3;
        if next == self.read_index.load(Ordering::Relaxed) {
            // Skip if readers are using next buffer
            return;
        }
        
        // Write to buffer
        if let Some(mut guard) = self.buffers[next].try_write() {
            *guard = data;
            self.write_index.store(next, Ordering::Release);
        }
    }
    
    /// Read latest data (GUI thread, multiple readers OK).
    fn read(&self) -> T {
        let idx = self.write_index.load(Ordering::Acquire);
        self.read_index.store(idx, Ordering::Relaxed);
        self.buffers[idx].read().clone()
    }
}

impl SharedVisualizationBuffer {
    /// Write from audio thread (never blocks).
    pub fn write_audio(&self, left: &[f32], right: &[f32]) {
        // Only process if someone is listening
        if self.subscriber_count.load(Ordering::Relaxed) == 0 {
            return;
        }
        
        // Update atomic levels
        let peak_l = left.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        let peak_r = right.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        self.peak_left.store(peak_l);
        self.peak_right.store(peak_r);
        
        // Downsample waveform for visualization
        let waveform = WaveformData::from_samples(left, right, 512);
        self.waveform.write(waveform);
    }
    
    /// Read from GUI thread (multiple readers OK).
    pub fn read_waveform(&self) -> WaveformData {
        self.waveform.read()
    }
    
    /// Subscribe/unsubscribe for backpressure.
    pub fn subscribe(&self) {
        self.subscriber_count.fetch_add(1, Ordering::Relaxed);
    }
    
    pub fn unsubscribe(&self) {
        self.subscriber_count.fetch_sub(1, Ordering::Relaxed);
    }
}
```

### 8.9 Komplett Flödesdiagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              AUDIO THREAD                                    │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                         SynthEngine                                  │   │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────────┐   │   │
│  │  │    Voices    │  │   Effects    │  │  SharedVisualizationBuffer│   │   │
│  │  └──────────────┘  └──────────────┘  └──────────────────────────┘   │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                    │                                         │
│                         Ring Buffer (Commands/Events)                        │
└────────────────────────────────────┼─────────────────────────────────────────┘
                                     │
┌────────────────────────────────────┼─────────────────────────────────────────┐
│                              MAIN THREAD                                     │
│                                    │                                         │
│  ┌─────────────────────────────────▼───────────────────────────────────┐   │
│  │                           EngineHub                                  │   │
│  │  ┌──────────────────────────────────────────────────────────────┐   │   │
│  │  │                    SharedEngineState                          │   │   │
│  │  │  ┌────────────┐ ┌──────────────┐ ┌─────────────────────────┐ │   │   │
│  │  │  │  Meters    │ │  Parameters  │ │    SharedGraphState     │ │   │   │
│  │  │  │  (Atomic)  │ │  (DashMap)   │ │  (modules, connections) │ │   │   │
│  │  │  └────────────┘ └──────────────┘ └─────────────────────────┘ │   │   │
│  │  └──────────────────────────────────────────────────────────────┘   │   │
│  │                                                                      │   │
│  │  ┌─────────────────────┐    ┌──────────────────────────────────┐   │   │
│  │  │   Command Aggregator │    │   Event Broadcaster (broadcast) │   │   │
│  │  │   (mpsc receiver)    │    │                                  │   │   │
│  │  └─────────────────────┘    └──────────────────────────────────┘   │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
│                                    │                                         │
│              ┌─────────────────────┼─────────────────────┐                  │
│              │                     │                     │                  │
│              ▼                     ▼                     ▼                  │
│  ┌───────────────────┐ ┌───────────────────┐ ┌───────────────────┐         │
│  │   ClientHandle    │ │   ClientHandle    │ │   ClientHandle    │         │
│  │   (Local GUI 1)   │ │   (Local GUI 2)   │ │ (Network/WebSocket)│         │
│  └─────────┬─────────┘ └─────────┬─────────┘ └─────────┬─────────┘         │
│            │                     │                     │                    │
└────────────┼─────────────────────┼─────────────────────┼────────────────────┘
             │                     │                     │
             ▼                     ▼                     ▼
      ┌────────────┐        ┌────────────┐        ┌────────────┐
      │  GUI #1    │        │  GUI #2    │        │  Browser   │
      │  (egui)    │        │  (iced)    │        │  (WASM)    │
      └────────────┘        └────────────┘        └────────────┘
```

### 8.10 Migration Path

**Steg 1: Behåll bakåtkompatibilitet**
```rust
impl EngineHub {
    /// For single-GUI mode, create a direct handle (backwards compatible).
    pub fn create_legacy_handle(&self) -> EngineHandle {
        // Wrap ClientHandle i EngineHandle interface
        let client = self.connect("legacy".into(), ClientType::LocalGui);
        EngineHandle::from_client(client)
    }
}
```

**Steg 2: Gradvis migration**
```rust
// Gammal kod fungerar fortfarande
let (engine, handle) = SynthEngine::new();

// Ny kod använder hub
let (engine, hub) = SynthEngine::with_hub();
let gui1 = hub.connect("Main".into(), ClientType::LocalGui);
let gui2 = hub.connect("Mixer".into(), ClientType::LocalGui);
```

---

## Del 9: Sammanfattning av Nya Features

### Multi-GUI Arkitektur
- ✅ **EngineHub** - Central distributor för flera klienter
- ✅ **ClientHandle** - Kloningsbar handle per GUI
- ✅ **broadcast::channel** - Events till alla klienter samtidigt
- ✅ **mpsc aggregation** - Commands från alla klienter
- ✅ **SharedEngineState** - Lock-free delad state via `Arc`
- ✅ **Konflikthantering** - Last-write-wins, locking, merge
- ✅ **Permissions** - Per-klient rättigheter
- ✅ **Network support** - WebSocket för remote GUI

### Kommunikation
- ✅ Multi-priority event channels
- ✅ Transaktionell command batching
- ✅ Dropped event tracking
- ✅ Full graph state synkronisering

### Modul-Visualisering
- ✅ Connectivity status (disconnected/partial/connected/bypassed)
- ✅ Opacity och färgkodning baserat på status
- ✅ Glow-effekt för aktiva moduler
- ✅ Per-modul mini-meters
- ✅ Error overlay och flash

### Kabel-Visualisering
- ✅ Signal level-baserad färg
- ✅ Animerad signal flow
- ✅ Tjocklek baserad på aktivitet

### Diagnostik
- ✅ Per-modul CPU tracking
- ✅ Voice stealing notifications
- ✅ Latency warnings
- ✅ Buffer underrun reporting

### API-förbättringar
```rust
// === Multi-GUI API ===

// Skapa engine med hub
let (engine, hub) = SynthEngine::with_hub();

// Skapa flera GUI-handles
let main_gui = hub.connect("Main Editor".into(), ClientType::LocalGui);
let mixer_gui = hub.connect("Mixer View".into(), ClientType::LocalGui);
let network_gui = hub.connect("iPad Remote".into(), ClientType::NetworkGui);

// Alla får samma events via broadcast
tokio::spawn(async move {
    while let Ok(event) = main_gui.recv_event().await {
        // Handle in main GUI
    }
});

// Alla kan skicka commands
mixer_gui.send(EngineCommand::SetMasterVolume(0.8)).await?;

// Alla har tillgång till delad state
let is_connected = main_gui.graph_state().is_live(module_id);

// === Legacy single-GUI (bakåtkompatibel) ===
let handle = hub.create_legacy_handle();
handle.send(EngineCommand::NoteOn { ... });

// === Batch operations ===
let mut batch = CommandBatch::new();
batch.add(EngineCommand::ClearAllModules);
batch.add(EngineCommand::AddModule { ... });
batch.add(EngineCommand::Connect { ... });
main_gui.send_batch(batch).await?;

// === Connectivity check ===
if shared_state.connectivity(module_id) == ModuleConnectivityStatus::Disconnected {
    show_warning_indicator();
}
```

---

## Del 10: Implementeringsordning

### Fas 1: Grundläggande Multi-GUI (1-2 veckor)
1. `SharedEngineState` med atomics och DashMap
2. `EngineHub` med command aggregation
3. `ClientHandle` med broadcast events
4. Migrera befintlig `EngineHandle` till `ClientHandle`

### Fas 2: Graf-synkronisering (1 vecka)
5. `SharedGraphState` med connectivity tracking
6. `ModuleConnectivityStatus` enum
7. Live module detection algorithm
8. Event broadcasting för topology changes

### Fas 3: Visuell Feedback (1 vecka)
9. `ModuleVisualState` med animationer
10. `CableVisualState` med signal flow
11. `MiniMeter` per modul
12. Integration i GUI framework

### Fas 4: Avancerade Features (1-2 veckor)
13. Konflikthantering (locking strategy)
14. Per-modul CPU tracking
15. Transaktionellt command batching
16. Network GUI via WebSocket

### Fas 5: Polish (1 vecka)
17. Permission system
18. Error visualization
19. Diagnostik events
20. Dokumentation och tester

---

## Del 11: Beroenden

```toml
[dependencies]
# Existing
ringbuf = "0.3"
parking_lot = "0.12"

# New for multi-GUI
dashmap = "5.5"           # Concurrent HashMap
tokio = { version = "1", features = ["sync", "rt-multi-thread", "net"] }
tokio-tungstenite = "0.21"  # WebSocket för network GUI
serde = { version = "1", features = ["derive"] }
serde_json = "1"          # För network serialization
```
