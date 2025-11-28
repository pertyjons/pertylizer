//! Patch file format for saving and loading synthesizer configurations.
//!
//! Patches contain:
//! - Module configurations (type, position, parameters)
//! - Connections between modules
//! - Global settings (master volume, BPM, etc.)
//!
//! ## Parameter Format
//! 
//! Parameters use typed values that map directly to the engine's TypedValue system:
//! - Waveforms: `"sine"`, `"triangle"`, `"sawtooth"`, `"square"`, `"pulse"`, `"noise"`
//! - LFO Waveforms: `"sine"`, `"triangle"`, `"sawtooth"`, `"square"`, `"sample_and_hold"`
//! - Filter Modes: `"lowpass"`, `"highpass"`, `"bandpass"`, `"notch"`, `"peak"`
//! - Delay Modes: `"mono"`, `"stereo"`, `"ping_pong"`
//! - Distortion Modes: `"soft_clip"`, `"hard_clip"`, `"tube"`, `"foldback"`, `"bitcrush"`

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use serde::{Serialize, Deserialize};

use crate::engine::ModuleId;
use crate::engine::graph::Connection;

/// A complete synthesizer patch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patch {
    /// Patch name.
    pub name: String,
    /// Author name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Patch version.
    #[serde(default = "default_version")]
    pub version: String,
    /// Description of the patch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Detailed explanation of how the patch works.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Tags for categorization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Module configurations.
    pub modules: Vec<ModuleState>,
    /// Connections between modules.
    pub connections: Vec<ConnectionState>,
    /// Global settings.
    #[serde(default)]
    pub settings: PatchSettings,
}

fn default_version() -> String {
    "1.0".to_string()
}

/// State of a single module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleState {
    /// Module ID.
    pub id: u32,
    /// Module type (oscillator, filter, envelope, etc.).
    #[serde(rename = "type")]
    pub module_type: ModuleType,
    /// Position in the rack view.
    pub position: (f32, f32),
    /// Parameter values.
    #[serde(default)]
    pub parameters: HashMap<String, ParamValue>,
}

/// Module types that can be saved/loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleType {
    Oscillator,
    Filter,
    Envelope,
    Lfo,
    Amplifier,
    Mixer,
    StereoOutput,
    Delay,
    Reverb,
    Distortion,
    Chorus,
    // Visualizers
    Oscilloscope,
    LevelMeter,
}

impl ModuleType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Oscillator => "oscillator",
            Self::Filter => "filter",
            Self::Envelope => "envelope",
            Self::Lfo => "lfo",
            Self::Amplifier => "amplifier",
            Self::Mixer => "mixer",
            Self::StereoOutput => "stereo_output",
            Self::Delay => "delay",
            Self::Reverb => "reverb",
            Self::Distortion => "distortion",
            Self::Chorus => "chorus",
            Self::Oscilloscope => "oscilloscope",
            Self::LevelMeter => "level_meter",
        }
    }
}

/// Parameter value for serialization.
/// Supports all types including string choices for waveforms, filter modes, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Float(f32),
    Int(i32),
    Bool(bool),
    Choice(String),
}


/// Connection between two modules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionState {
    /// Source (module_id, port_name).
    pub from: (u32, String),
    /// Destination (module_id, port_name).
    pub to: (u32, String),
}

impl From<&Connection> for ConnectionState {
    fn from(c: &Connection) -> Self {
        Self {
            from: (c.from_module.0, c.from_port.clone()),
            to: (c.to_module.0, c.to_port.clone()),
        }
    }
}

impl From<&ConnectionState> for Connection {
    fn from(c: &ConnectionState) -> Self {
        Connection::new(
            ModuleId::new(c.from.0),
            &c.from.1,
            ModuleId::new(c.to.0),
            &c.to.1,
        )
    }
}

/// Global patch settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchSettings {
    /// Master volume (0.0 - 1.0).
    #[serde(default = "default_master_volume")]
    pub master_volume: f32,
    /// Tempo in BPM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f32>,
    /// Octave offset for keyboard.
    #[serde(default)]
    pub octave_offset: i32,
    /// Glide/portamento time in seconds (0.0 = off).
    #[serde(default)]
    pub glide_time: f32,
}

fn default_master_volume() -> f32 {
    0.8
}

impl Default for PatchSettings {
    fn default() -> Self {
        Self {
            master_volume: 0.8,
            bpm: Some(120.0),
            octave_offset: 0,
            glide_time: 0.0,
        }
    }
}

impl Patch {
    /// Create a new empty patch.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            author: None,
            version: "1.0".to_string(),
            description: None,
            notes: None,
            tags: Vec::new(),
            modules: Vec::new(),
            connections: Vec::new(),
            settings: PatchSettings::default(),
        }
    }

    /// Load a patch from a JSON file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PatchError> {
        let content = fs::read_to_string(path.as_ref())
            .map_err(|e| PatchError::Io(e.to_string()))?;
        
        serde_json::from_str(&content)
            .map_err(|e| PatchError::Parse(e.to_string()))
    }

    /// Save the patch to a JSON file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), PatchError> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| PatchError::Serialize(e.to_string()))?;
        
        fs::write(path.as_ref(), content)
            .map_err(|e| PatchError::Io(e.to_string()))
    }

    /// Add a module to the patch.
    pub fn add_module(&mut self, module: ModuleState) {
        self.modules.push(module);
    }

    /// Add a connection to the patch.
    pub fn add_connection(&mut self, from_id: u32, from_port: &str, to_id: u32, to_port: &str) {
        self.connections.push(ConnectionState {
            from: (from_id, from_port.to_string()),
            to: (to_id, to_port.to_string()),
        });
    }

    /// Get the next available module ID.
    pub fn next_module_id(&self) -> u32 {
        self.modules.iter().map(|m| m.id).max().unwrap_or(0) + 1
    }
}

/// Errors that can occur when loading/saving patches.
#[derive(Debug, Clone)]
pub enum PatchError {
    Io(String),
    Parse(String),
    Serialize(String),
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::Parse(e) => write!(f, "Parse error: {}", e),
            Self::Serialize(e) => write!(f, "Serialize error: {}", e),
        }
    }
}

impl std::error::Error for PatchError {}

// ============================================================================
// MODULE BUILDER
// ============================================================================

/// Helper to create a module state with parameters.
pub struct ModuleBuilder {
    state: ModuleState,
}

impl ModuleBuilder {
    pub fn new(id: u32, module_type: ModuleType) -> Self {
        Self {
            state: ModuleState {
                id,
                module_type,
                position: (0.0, 0.0),
                parameters: HashMap::new(),
            },
        }
    }

    pub fn position(mut self, x: f32, y: f32) -> Self {
        self.state.position = (x, y);
        self
    }

    /// Set a float parameter.
    pub fn param_f(mut self, name: &str, value: f32) -> Self {
        self.state.parameters.insert(name.to_string(), ParamValue::Float(value));
        self
    }

    /// Set an integer parameter.
    pub fn param_i(mut self, name: &str, value: i32) -> Self {
        self.state.parameters.insert(name.to_string(), ParamValue::Int(value));
        self
    }

    /// Set a boolean parameter.
    pub fn param_b(mut self, name: &str, value: bool) -> Self {
        self.state.parameters.insert(name.to_string(), ParamValue::Bool(value));
        self
    }

    /// Set a choice/enum parameter (waveform, filter mode, etc.).
    pub fn param_choice(mut self, name: &str, value: &str) -> Self {
        self.state.parameters.insert(name.to_string(), ParamValue::Choice(value.to_string()));
        self
    }

    /// Convenience: set waveform for oscillator.
    pub fn waveform(self, wf: &str) -> Self {
        self.param_choice("waveform", wf)
    }

    /// Convenience: set filter mode.
    pub fn filter_mode(self, mode: &str) -> Self {
        self.param_choice("filter_type", mode)
    }

    /// Convenience: set delay mode.
    pub fn delay_mode(self, mode: &str) -> Self {
        self.param_choice("mode", mode)
    }

    /// Convenience: set distortion mode.
    pub fn distortion_mode(self, mode: &str) -> Self {
        self.param_choice("mode", mode)
    }

    pub fn build(self) -> ModuleState {
        self.state
    }
}

// ============================================================================
// EXAMPLE PATCHES
// ============================================================================

/// Get all example patches.
pub fn example_patches() -> Vec<Patch> {
    vec![
        patch_deep_space_pad(),
        patch_aggressive_bass(),
        patch_vintage_lead(),
        patch_ambient_keys(),
        patch_drum_kick(),
        patch_drum_snare(),
        patch_drum_hihat(),
        patch_pluck_synth(),
        patch_fm_bell(),
        patch_noise_sweep(),
    ]
}

// ----------------------------------------------------------------------------
// PAD SOUNDS
// ----------------------------------------------------------------------------

/// Deep Space Pad - Evolving atmospheric pad with multiple modulation sources.
pub fn patch_deep_space_pad() -> Patch {
    let mut patch = Patch::new("Deep Space Pad");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Evolving atmospheric pad with rich modulation.".to_string());
    patch.notes = Some(r#"
SIGNAL FLOW:
Two detuned sawtooth oscillators create a rich, full sound. OSC1 is the main 
oscillator while OSC2 is slightly detuned (+7 cents) for natural chorus effect.

Both oscillators feed into a lowpass filter with moderate resonance, creating
warmth. The filter cutoff is modulated by both an envelope (for attack brightness)
and a slow LFO (for movement).

MODULATION:
- LFO1 (0.1 Hz): Slowly modulates filter cutoff for evolving texture
- LFO2 (0.08 Hz): Modulates OSC2 pitch for subtle detuning variation
- Filter Envelope: Opens filter on attack, then settles

The chorus effect adds width and dimension, while the reverb places the sound
in a large space. Long attack/release times create the pad-like quality.

TRY: Play sustained chords in the low-to-mid range. Layer with arpeggios.
"#.to_string());
    patch.tags = vec!["pad".into(), "ambient".into(), "atmospheric".into(), "evolving".into()];

    // OSC1 - Main sawtooth
    patch.add_module(ModuleBuilder::new(1, ModuleType::Oscillator)
        .position(50.0, 50.0)
        .waveform("sawtooth")
        .param_f("level", 0.5)
        .param_f("detune", 0.0)
        .build());

    // OSC2 - Detuned sawtooth
    patch.add_module(ModuleBuilder::new(2, ModuleType::Oscillator)
        .position(50.0, 200.0)
        .waveform("sawtooth")
        .param_f("level", 0.5)
        .param_f("detune", 7.0)
        .build());

    // Mixer for oscillators
    patch.add_module(ModuleBuilder::new(3, ModuleType::Mixer)
        .position(250.0, 100.0)
        .param_f("level", 0.8)
        .build());

    // Filter - Lowpass with resonance
    patch.add_module(ModuleBuilder::new(4, ModuleType::Filter)
        .position(450.0, 100.0)
        .filter_mode("lowpass")
        .param_f("cutoff", 800.0)
        .param_f("resonance", 0.35)
        .build());

    // Amp Envelope - Slow pad envelope
    patch.add_module(ModuleBuilder::new(5, ModuleType::Envelope)
        .position(50.0, 400.0)
        .param_f("attack", 1.5)
        .param_f("decay", 0.5)
        .param_f("sustain", 0.7)
        .param_f("release", 3.0)
        .build());

    // Filter Envelope - Brighter on attack
    patch.add_module(ModuleBuilder::new(6, ModuleType::Envelope)
        .position(250.0, 400.0)
        .param_f("attack", 0.8)
        .param_f("decay", 1.5)
        .param_f("sustain", 0.3)
        .param_f("release", 2.0)
        .build());

    // LFO1 - Filter modulation
    patch.add_module(ModuleBuilder::new(7, ModuleType::Lfo)
        .position(450.0, 400.0)
        .waveform("sine")
        .param_f("rate", 0.1)
        .param_f("depth", 0.4)
        .build());

    // LFO2 - Pitch modulation for OSC2
    patch.add_module(ModuleBuilder::new(8, ModuleType::Lfo)
        .position(650.0, 400.0)
        .waveform("triangle")
        .param_f("rate", 0.08)
        .param_f("depth", 0.1)
        .build());

    // Amplifier
    patch.add_module(ModuleBuilder::new(9, ModuleType::Amplifier)
        .position(650.0, 100.0)
        .param_f("level", 0.6)
        .build());

    // Chorus - For width
    patch.add_module(ModuleBuilder::new(100, ModuleType::Chorus)
        .position(850.0, 100.0)
        .param_f("rate", 0.5)
        .param_f("depth", 0.4)
        .param_f("mix", 0.35)
        .build());

    // Reverb - Large space
    patch.add_module(ModuleBuilder::new(101, ModuleType::Reverb)
        .position(1050.0, 100.0)
        .param_f("room_size", 0.85)
        .param_f("damping", 0.3)
        .param_f("mix", 0.4)
        .build());

    // Connections
    patch.add_connection(1, "out", 3, "in1");
    patch.add_connection(2, "out", 3, "in2");
    patch.add_connection(3, "out", 4, "in");
    patch.add_connection(4, "out", 9, "in");
    patch.add_connection(5, "out", 9, "cv");
    patch.add_connection(6, "out", 4, "cutoff_cv");
    patch.add_connection(7, "out", 4, "cutoff_cv");
    patch.add_connection(8, "out", 2, "fm");
    patch.add_connection(9, "out", 100, "in_l");
    patch.add_connection(100, "out_l", 101, "in_l");
    patch.add_connection(100, "out_r", 101, "in_r");

    patch.settings.octave_offset = -1;
    patch
}

// ----------------------------------------------------------------------------
// BASS SOUNDS
// ----------------------------------------------------------------------------

/// Aggressive Bass - Punchy, distorted bass with filter movement.
pub fn patch_aggressive_bass() -> Patch {
    let mut patch = Patch::new("Aggressive Bass");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Punchy, aggressive bass with distortion and filter sweep.".to_string());
    patch.notes = Some(r#"
SIGNAL FLOW:
A square wave provides the fundamental punch and harmonic content. The square
wave's odd harmonics give the bass its characteristic hollow, powerful sound.

The signal passes through a resonant lowpass filter that sweeps down from a 
higher cutoff on each note attack, creating the classic "zap" bass sound.
High resonance adds edge and aggression.

DISTORTION:
The tube distortion after the filter adds warmth and harmonics, making the
bass cut through a mix. The drive is set moderately to add grit without 
losing the low-end punch.

ENVELOPE DESIGN:
- Fast attack (2ms) for immediate punch
- Short decay to a moderate sustain keeps energy
- Filter envelope has longer decay for the "wah" sweep effect

TRY: Play single notes with space between them. Works great for EDM/dubstep
style bass lines. Try different octaves for different characters.
"#.to_string());
    patch.tags = vec!["bass".into(), "aggressive".into(), "edm".into(), "punchy".into()];

    // OSC - Square wave for punch
    patch.add_module(ModuleBuilder::new(1, ModuleType::Oscillator)
        .position(50.0, 50.0)
        .waveform("square")
        .param_f("level", 0.9)
        .build());

    // Filter - Resonant lowpass
    patch.add_module(ModuleBuilder::new(2, ModuleType::Filter)
        .position(250.0, 50.0)
        .filter_mode("lowpass")
        .param_f("cutoff", 400.0)
        .param_f("resonance", 0.6)
        .build());

    // Amp Envelope - Punchy
    patch.add_module(ModuleBuilder::new(3, ModuleType::Envelope)
        .position(50.0, 300.0)
        .param_f("attack", 0.002)
        .param_f("decay", 0.15)
        .param_f("sustain", 0.6)
        .param_f("release", 0.1)
        .build());

    // Filter Envelope - Sweep
    patch.add_module(ModuleBuilder::new(4, ModuleType::Envelope)
        .position(250.0, 300.0)
        .param_f("attack", 0.001)
        .param_f("decay", 0.25)
        .param_f("sustain", 0.2)
        .param_f("release", 0.1)
        .build());

    // Amplifier
    patch.add_module(ModuleBuilder::new(5, ModuleType::Amplifier)
        .position(450.0, 50.0)
        .param_f("level", 0.8)
        .build());

    // Distortion - Tube warmth
    patch.add_module(ModuleBuilder::new(100, ModuleType::Distortion)
        .position(650.0, 50.0)
        .distortion_mode("tube")
        .param_f("drive", 0.5)
        .param_f("tone", 0.4)
        .param_f("mix", 0.6)
        .build());

    // Connections
    patch.add_connection(1, "out", 2, "in");
    patch.add_connection(2, "out", 5, "in");
    patch.add_connection(3, "out", 5, "cv");
    patch.add_connection(4, "out", 2, "cutoff_cv");
    patch.add_connection(5, "out", 100, "in_l");

    patch.settings.octave_offset = -2;
    patch
}

// ----------------------------------------------------------------------------
// LEAD SOUNDS
// ----------------------------------------------------------------------------

/// Vintage Lead - Classic analog-style mono lead with vibrato.
pub fn patch_vintage_lead() -> Patch {
    let mut patch = Patch::new("Vintage Lead");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Classic analog-style mono lead with vibrato and delay.".to_string());
    patch.notes = Some(r#"
SIGNAL FLOW:
Two oscillators create the core sound: a sawtooth for brightness and a 
pulse wave (with PWM) for movement and thickness. The pulse width is 
modulated by a slow LFO, creating the classic "breathing" analog sound.

The mixed signal passes through a resonant lowpass filter. The filter
envelope provides the attack "bite" that helps notes stand out.

VIBRATO:
A dedicated vibrato LFO modulates OSC1's pitch at about 5.5 Hz. The depth
is subtle (1.5%) to add expression without being overbearing. Classic 
lead synths use this technique for a more human, expressive quality.

EFFECTS:
The delay adds rhythmic interest and fills space between notes. The 
ping-pong mode creates stereo width. Moderate feedback creates trails
without overwhelming the dry signal.

TRY: Play melodic lines in the upper register. The vibrato adds expression 
to held notes. Use pitch bends for extra expressiveness.
"#.to_string());
    patch.tags = vec!["lead".into(), "vintage".into(), "analog".into(), "mono".into()];

    // OSC1 - Sawtooth
    patch.add_module(ModuleBuilder::new(1, ModuleType::Oscillator)
        .position(50.0, 50.0)
        .waveform("sawtooth")
        .param_f("level", 0.6)
        .build());

    // OSC2 - Pulse with PWM
    patch.add_module(ModuleBuilder::new(2, ModuleType::Oscillator)
        .position(50.0, 200.0)
        .waveform("pulse")
        .param_f("level", 0.4)
        .param_f("pulse_width", 0.3)
        .build());

    // Mixer
    patch.add_module(ModuleBuilder::new(3, ModuleType::Mixer)
        .position(250.0, 100.0)
        .param_f("level", 0.85)
        .build());

    // Filter
    patch.add_module(ModuleBuilder::new(4, ModuleType::Filter)
        .position(450.0, 100.0)
        .filter_mode("lowpass")
        .param_f("cutoff", 2000.0)
        .param_f("resonance", 0.4)
        .build());

    // Amp Envelope
    patch.add_module(ModuleBuilder::new(5, ModuleType::Envelope)
        .position(50.0, 400.0)
        .param_f("attack", 0.005)
        .param_f("decay", 0.1)
        .param_f("sustain", 0.8)
        .param_f("release", 0.2)
        .build());

    // Filter Envelope
    patch.add_module(ModuleBuilder::new(6, ModuleType::Envelope)
        .position(250.0, 400.0)
        .param_f("attack", 0.002)
        .param_f("decay", 0.2)
        .param_f("sustain", 0.4)
        .param_f("release", 0.15)
        .build());

    // Vibrato LFO
    patch.add_module(ModuleBuilder::new(7, ModuleType::Lfo)
        .position(450.0, 400.0)
        .waveform("sine")
        .param_f("rate", 5.5)
        .param_f("depth", 0.015)
        .build());

    // PWM LFO
    patch.add_module(ModuleBuilder::new(8, ModuleType::Lfo)
        .position(650.0, 400.0)
        .waveform("triangle")
        .param_f("rate", 0.4)
        .param_f("depth", 0.35)
        .build());

    // Amplifier
    patch.add_module(ModuleBuilder::new(9, ModuleType::Amplifier)
        .position(650.0, 100.0)
        .param_f("level", 0.75)
        .build());

    // Delay
    patch.add_module(ModuleBuilder::new(100, ModuleType::Delay)
        .position(850.0, 100.0)
        .delay_mode("ping_pong")
        .param_f("time", 0.35)
        .param_f("feedback", 0.4)
        .param_f("mix", 0.3)
        .build());

    // Connections
    patch.add_connection(1, "out", 3, "in1");
    patch.add_connection(2, "out", 3, "in2");
    patch.add_connection(3, "out", 4, "in");
    patch.add_connection(4, "out", 9, "in");
    patch.add_connection(5, "out", 9, "cv");
    patch.add_connection(6, "out", 4, "cutoff_cv");
    patch.add_connection(7, "out", 1, "fm");
    patch.add_connection(8, "out", 2, "pwm");
    patch.add_connection(9, "out", 100, "in_l");

    patch.settings.octave_offset = 1;
    patch
}

// ----------------------------------------------------------------------------
// KEYS SOUNDS
// ----------------------------------------------------------------------------

/// Ambient Keys - Soft, dreamy electric piano with shimmer.
pub fn patch_ambient_keys() -> Patch {
    let mut patch = Patch::new("Ambient Keys");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Soft, dreamy electric piano with shimmer and reverb.".to_string());
    patch.notes = Some(r#"
SIGNAL FLOW:
A triangle wave provides a soft, pure fundamental - similar to a tine-based
electric piano like a Rhodes. The triangle has fewer harmonics than a saw
or square, giving a gentler character.

The filter is set relatively open with low resonance, just gently rolling
off the highest frequencies for warmth without dulling the sound.

DYNAMICS:
The amp envelope mimics a piano: instant attack, moderate decay to a 
lower sustain level (simulating the tine's natural decay), and medium
release for notes that fade naturally.

The filter envelope adds subtle brightness on attack - that characteristic
"bell" of an electric piano - then settles to a warmer sustained tone.

EFFECTS CHAIN:
1. Chorus adds subtle width and movement
2. Heavy reverb creates the ambient, dreamy quality
3. Together they create a lush, spacious sound

TRY: Play soft chords, let them ring and overlap. Works beautifully for
ambient music, ballads, or as a bed under other instruments.
"#.to_string());
    patch.tags = vec!["keys".into(), "ambient".into(), "electric_piano".into(), "dreamy".into()];

    // OSC - Triangle for soft tone
    patch.add_module(ModuleBuilder::new(1, ModuleType::Oscillator)
        .position(50.0, 50.0)
        .waveform("triangle")
        .param_f("level", 0.8)
        .build());

    // Filter
    patch.add_module(ModuleBuilder::new(2, ModuleType::Filter)
        .position(250.0, 50.0)
        .filter_mode("lowpass")
        .param_f("cutoff", 3000.0)
        .param_f("resonance", 0.15)
        .build());

    // Amp Envelope
    patch.add_module(ModuleBuilder::new(3, ModuleType::Envelope)
        .position(50.0, 300.0)
        .param_f("attack", 0.003)
        .param_f("decay", 0.8)
        .param_f("sustain", 0.4)
        .param_f("release", 0.6)
        .build());

    // Filter Envelope
    patch.add_module(ModuleBuilder::new(4, ModuleType::Envelope)
        .position(250.0, 300.0)
        .param_f("attack", 0.001)
        .param_f("decay", 0.4)
        .param_f("sustain", 0.2)
        .param_f("release", 0.3)
        .build());

    // Subtle tremolo LFO
    patch.add_module(ModuleBuilder::new(5, ModuleType::Lfo)
        .position(450.0, 300.0)
        .waveform("sine")
        .param_f("rate", 4.0)
        .param_f("depth", 0.06)
        .build());

    // Amplifier
    patch.add_module(ModuleBuilder::new(6, ModuleType::Amplifier)
        .position(450.0, 50.0)
        .param_f("level", 0.65)
        .build());

    // Chorus
    patch.add_module(ModuleBuilder::new(100, ModuleType::Chorus)
        .position(650.0, 50.0)
        .param_f("rate", 0.7)
        .param_f("depth", 0.3)
        .param_f("mix", 0.3)
        .build());

    // Reverb - Large and lush
    patch.add_module(ModuleBuilder::new(101, ModuleType::Reverb)
        .position(850.0, 50.0)
        .param_f("room_size", 0.9)
        .param_f("damping", 0.25)
        .param_f("mix", 0.5)
        .build());

    // Connections
    patch.add_connection(1, "out", 2, "in");
    patch.add_connection(2, "out", 6, "in");
    patch.add_connection(3, "out", 6, "cv");
    patch.add_connection(4, "out", 2, "cutoff_cv");
    patch.add_connection(5, "out", 6, "cv");
    patch.add_connection(6, "out", 100, "in_l");
    patch.add_connection(100, "out_l", 101, "in_l");
    patch.add_connection(100, "out_r", 101, "in_r");

    patch
}

// ----------------------------------------------------------------------------
// DRUM SOUNDS
// ----------------------------------------------------------------------------

/// Kick Drum - Classic electronic kick with punch.
pub fn patch_drum_kick() -> Patch {
    let mut patch = Patch::new("Kick Drum");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Punchy electronic kick drum with pitch sweep.".to_string());
    patch.notes = Some(r#"
SIGNAL FLOW:
The kick uses a sine wave oscillator for its pure, subby fundamental.
Electronic kicks are characterized by a pitch sweep - the tone starts
higher and rapidly drops to the fundamental.

The oscillator frequency is modulated by a fast envelope that creates
this pitch sweep. The envelope attacks instantly and decays very quickly
(~50ms), sweeping the pitch from around 150Hz down to the base frequency.

AMPLITUDE:
A separate envelope controls volume with:
- Instant attack (1ms) for the initial transient "click"
- Short decay (~150ms) for the body of the kick
- No sustain - kicks are purely percussive
- Minimal release for tight sound

CHARACTERISTICS:
- Lower base frequencies = deeper, subby kicks
- Faster pitch decay = tighter, punchier kicks
- Slower pitch decay = more "boom", 808-style

TRY: Play single hits. Adjust the pitch envelope decay for different 
kick characters. Works well in the lowest octave.
"#.to_string());
    patch.tags = vec!["drum".into(), "kick".into(), "percussion".into(), "808".into()];

    // OSC - Sine for pure sub
    patch.add_module(ModuleBuilder::new(1, ModuleType::Oscillator)
        .position(50.0, 50.0)
        .waveform("sine")
        .param_f("level", 1.0)
        .build());

    // Pitch Envelope - Fast sweep
    patch.add_module(ModuleBuilder::new(2, ModuleType::Envelope)
        .position(50.0, 250.0)
        .param_f("attack", 0.001)
        .param_f("decay", 0.05)
        .param_f("sustain", 0.0)
        .param_f("release", 0.01)
        .build());

    // Amp Envelope
    patch.add_module(ModuleBuilder::new(3, ModuleType::Envelope)
        .position(250.0, 250.0)
        .param_f("attack", 0.001)
        .param_f("decay", 0.15)
        .param_f("sustain", 0.0)
        .param_f("release", 0.05)
        .build());

    // Amplifier
    patch.add_module(ModuleBuilder::new(4, ModuleType::Amplifier)
        .position(250.0, 50.0)
        .param_f("level", 0.9)
        .build());

    // Soft clip for warmth
    patch.add_module(ModuleBuilder::new(100, ModuleType::Distortion)
        .position(450.0, 50.0)
        .distortion_mode("soft_clip")
        .param_f("drive", 0.2)
        .param_f("mix", 0.3)
        .build());

    // Connections
    patch.add_connection(1, "out", 4, "in");
    patch.add_connection(2, "out", 1, "fm");
    patch.add_connection(3, "out", 4, "cv");
    patch.add_connection(4, "out", 100, "in_l");

    patch.settings.octave_offset = -2;
    patch
}

/// Snare Drum - Punchy electronic snare with noise.
pub fn patch_drum_snare() -> Patch {
    let mut patch = Patch::new("Snare Drum");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Electronic snare with tuned body and noise snap.".to_string());
    patch.notes = Some(r#"
SIGNAL FLOW:
A snare drum has two components: the tuned "body" (from the drum head)
and the "snare" rattle (from the snare wires). This patch creates both.

BODY (OSC1 - Triangle):
The triangle wave provides a tuned, punchy body. A pitch envelope sweeps
from higher to lower, similar to a kick but faster, giving that characteristic
snare "pop".

SNARE/NOISE (OSC2 - Noise):
White noise through a bandpass filter creates the snare wire sound.
The bandpass centers around 5kHz with moderate resonance for a 
metallic, snappy character.

MIXING:
Both sounds are mixed together and share the same amp envelope.
The balance between body and noise determines the snare character:
- More body = deeper, more tonal
- More noise = brighter, snappier

TRY: Adjust the noise filter cutoff for different snare characters.
Higher cutoff = brighter, crisper. Lower = darker, thicker.
"#.to_string());
    patch.tags = vec!["drum".into(), "snare".into(), "percussion".into()];

    // OSC1 - Triangle for body
    patch.add_module(ModuleBuilder::new(1, ModuleType::Oscillator)
        .position(50.0, 50.0)
        .waveform("triangle")
        .param_f("level", 0.6)
        .build());

    // OSC2 - Noise for snare
    patch.add_module(ModuleBuilder::new(2, ModuleType::Oscillator)
        .position(50.0, 200.0)
        .waveform("noise")
        .param_f("level", 0.7)
        .build());

    // Noise Filter - Bandpass
    patch.add_module(ModuleBuilder::new(3, ModuleType::Filter)
        .position(250.0, 200.0)
        .filter_mode("bandpass")
        .param_f("cutoff", 5000.0)
        .param_f("resonance", 0.3)
        .build());

    // Mixer
    patch.add_module(ModuleBuilder::new(4, ModuleType::Mixer)
        .position(450.0, 100.0)
        .param_f("level", 0.8)
        .build());

    // Pitch Envelope
    patch.add_module(ModuleBuilder::new(5, ModuleType::Envelope)
        .position(50.0, 400.0)
        .param_f("attack", 0.001)
        .param_f("decay", 0.03)
        .param_f("sustain", 0.0)
        .param_f("release", 0.01)
        .build());

    // Amp Envelope
    patch.add_module(ModuleBuilder::new(6, ModuleType::Envelope)
        .position(250.0, 400.0)
        .param_f("attack", 0.001)
        .param_f("decay", 0.12)
        .param_f("sustain", 0.0)
        .param_f("release", 0.08)
        .build());

    // Amplifier
    patch.add_module(ModuleBuilder::new(7, ModuleType::Amplifier)
        .position(650.0, 100.0)
        .param_f("level", 0.75)
        .build());

    // Connections
    patch.add_connection(1, "out", 4, "in1");
    patch.add_connection(2, "out", 3, "in");
    patch.add_connection(3, "out", 4, "in2");
    patch.add_connection(4, "out", 7, "in");
    patch.add_connection(5, "out", 1, "fm");
    patch.add_connection(6, "out", 7, "cv");

    patch.settings.octave_offset = -1;
    patch
}

/// Hi-Hat - Metallic electronic hi-hat.
pub fn patch_drum_hihat() -> Patch {
    let mut patch = Patch::new("Hi-Hat");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Metallic electronic hi-hat with variable decay.".to_string());
    patch.notes = Some(r#"
SIGNAL FLOW:
Hi-hats are essentially filtered noise with very fast envelopes.
The metallic quality comes from highpass filtering that removes
the low frequencies, leaving only the bright, shimmery content.

FILTER:
A highpass filter at around 7-8kHz removes the "body" of the noise,
leaving the bright, metallic character. Higher resonance adds a 
slight ring/shimmer.

ENVELOPE:
The amp envelope is extremely fast:
- Instant attack (1ms)
- Very short decay (30-50ms for closed, 200ms+ for open)
- No sustain - hi-hats are purely percussive

CLOSED vs OPEN:
Adjust the decay time to switch between closed and open hi-hats:
- Closed: 30-50ms decay
- Open: 150-300ms decay

TRY: Play rapid 16th notes for closed hi-hat patterns. Longer notes
for open hi-hats. The filter cutoff affects brightness.
"#.to_string());
    patch.tags = vec!["drum".into(), "hihat".into(), "percussion".into(), "cymbal".into()];

    // OSC - Noise
    patch.add_module(ModuleBuilder::new(1, ModuleType::Oscillator)
        .position(50.0, 50.0)
        .waveform("noise")
        .param_f("level", 0.8)
        .build());

    // Filter - Highpass
    patch.add_module(ModuleBuilder::new(2, ModuleType::Filter)
        .position(250.0, 50.0)
        .filter_mode("highpass")
        .param_f("cutoff", 7500.0)
        .param_f("resonance", 0.4)
        .build());

    // Amp Envelope - Very short
    patch.add_module(ModuleBuilder::new(3, ModuleType::Envelope)
        .position(50.0, 300.0)
        .param_f("attack", 0.001)
        .param_f("decay", 0.05)
        .param_f("sustain", 0.0)
        .param_f("release", 0.03)
        .build());

    // Amplifier
    patch.add_module(ModuleBuilder::new(4, ModuleType::Amplifier)
        .position(450.0, 50.0)
        .param_f("level", 0.5)
        .build());

    // Connections
    patch.add_connection(1, "out", 2, "in");
    patch.add_connection(2, "out", 4, "in");
    patch.add_connection(3, "out", 4, "cv");

    patch
}

// ----------------------------------------------------------------------------
// PLUCK/BELL SOUNDS
// ----------------------------------------------------------------------------

/// Pluck Synth - Short, plucky synthesizer sound.
pub fn patch_pluck_synth() -> Patch {
    let mut patch = Patch::new("Pluck Synth");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Short, plucky synthesizer for arpeggios and sequences.".to_string());
    patch.notes = Some(r#"
SIGNAL FLOW:
A sawtooth wave provides rich harmonics that will be sculpted by the
filter. Sawtooth is ideal for plucks because it has all harmonics,
giving the filter plenty of material to shape.

FILTER CHARACTER:
The resonant lowpass filter is key to the pluck sound. The envelope
rapidly opens and closes the filter:
- Fast attack opens instantly
- Quick decay (100ms) creates the "pluck"
- Moderate resonance adds a slight "ping"

This mimics how physical plucked strings have bright attacks that
quickly mellow out.

AMP ENVELOPE:
Similarly shaped to the filter but with slightly longer decay,
allowing the filter's "pluck" to be heard before the volume fades.

EFFECTS:
The ping-pong delay adds rhythmic interest, perfect for arpeggios.
It turns single notes into cascading patterns.

TRY: Play arpeggiated patterns or sequences. The delay creates
instant complexity. Great for EDM, synthwave, and electronic pop.
"#.to_string());
    patch.tags = vec!["pluck".into(), "synth".into(), "arpeggio".into(), "sequence".into()];

    // OSC - Sawtooth
    patch.add_module(ModuleBuilder::new(1, ModuleType::Oscillator)
        .position(50.0, 50.0)
        .waveform("sawtooth")
        .param_f("level", 0.8)
        .build());

    // Filter
    patch.add_module(ModuleBuilder::new(2, ModuleType::Filter)
        .position(250.0, 50.0)
        .filter_mode("lowpass")
        .param_f("cutoff", 1000.0)
        .param_f("resonance", 0.45)
        .build());

    // Amp Envelope
    patch.add_module(ModuleBuilder::new(3, ModuleType::Envelope)
        .position(50.0, 300.0)
        .param_f("attack", 0.002)
        .param_f("decay", 0.2)
        .param_f("sustain", 0.0)
        .param_f("release", 0.15)
        .build());

    // Filter Envelope
    patch.add_module(ModuleBuilder::new(4, ModuleType::Envelope)
        .position(250.0, 300.0)
        .param_f("attack", 0.001)
        .param_f("decay", 0.1)
        .param_f("sustain", 0.1)
        .param_f("release", 0.1)
        .build());

    // Amplifier
    patch.add_module(ModuleBuilder::new(5, ModuleType::Amplifier)
        .position(450.0, 50.0)
        .param_f("level", 0.7)
        .build());

    // Delay
    patch.add_module(ModuleBuilder::new(100, ModuleType::Delay)
        .position(650.0, 50.0)
        .delay_mode("ping_pong")
        .param_f("time", 0.25)
        .param_f("feedback", 0.45)
        .param_f("mix", 0.35)
        .build());

    // Connections
    patch.add_connection(1, "out", 2, "in");
    patch.add_connection(2, "out", 5, "in");
    patch.add_connection(3, "out", 5, "cv");
    patch.add_connection(4, "out", 2, "cutoff_cv");
    patch.add_connection(5, "out", 100, "in_l");

    patch
}

/// FM Bell - Bright, bell-like FM synthesis sound.
pub fn patch_fm_bell() -> Patch {
    let mut patch = Patch::new("FM Bell");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Bright, metallic bell sound using FM synthesis.".to_string());
    patch.notes = Some(r#"
SIGNAL FLOW:
This patch creates a bell-like tone using FM (Frequency Modulation)
synthesis. In FM, one oscillator (the modulator) modulates the 
frequency of another (the carrier), creating complex harmonics.

FM BASICS:
- Carrier (OSC1): Sine wave - the sound you actually hear
- Modulator (OSC2): Sine wave - modulates OSC1's frequency
- When OSC2's output connects to OSC1's FM input, the carrier's
  frequency wobbles, creating sidebands (new harmonics)

BELL CHARACTER:
Bells have inharmonic partials (frequencies that aren't simple
multiples of the fundamental). FM naturally creates these when
the modulator and carrier have non-integer frequency ratios.

The amp envelope has instant attack and long decay, mimicking
how a struck bell rings and slowly fades.

REVERB:
Heavy reverb simulates the acoustic space where bells typically
exist (churches, towers) and extends the decay naturally.

TRY: Play single notes and let them ring. Higher notes sound
more "chime-like", lower notes more "gong-like".
"#.to_string());
    patch.tags = vec!["bell".into(), "fm".into(), "metallic".into(), "chime".into()];

    // OSC1 - Carrier (sine)
    patch.add_module(ModuleBuilder::new(1, ModuleType::Oscillator)
        .position(250.0, 50.0)
        .waveform("sine")
        .param_f("level", 0.7)
        .build());

    // OSC2 - Modulator (sine)
    patch.add_module(ModuleBuilder::new(2, ModuleType::Oscillator)
        .position(50.0, 50.0)
        .waveform("sine")
        .param_f("level", 0.5)
        .param_f("detune", 2.0)  // Slightly detuned for inharmonic partials
        .build());

    // Modulator Envelope - Controls FM depth
    patch.add_module(ModuleBuilder::new(3, ModuleType::Envelope)
        .position(50.0, 300.0)
        .param_f("attack", 0.001)
        .param_f("decay", 0.5)
        .param_f("sustain", 0.1)
        .param_f("release", 0.3)
        .build());

    // Amp Envelope - Bell-like
    patch.add_module(ModuleBuilder::new(4, ModuleType::Envelope)
        .position(250.0, 300.0)
        .param_f("attack", 0.001)
        .param_f("decay", 2.0)
        .param_f("sustain", 0.0)
        .param_f("release", 1.0)
        .build());

    // Amplifier
    patch.add_module(ModuleBuilder::new(5, ModuleType::Amplifier)
        .position(450.0, 50.0)
        .param_f("level", 0.6)
        .build());

    // Reverb - Large space
    patch.add_module(ModuleBuilder::new(100, ModuleType::Reverb)
        .position(650.0, 50.0)
        .param_f("room_size", 0.8)
        .param_f("damping", 0.2)
        .param_f("mix", 0.45)
        .build());

    // Connections
    patch.add_connection(2, "out", 1, "fm");
    patch.add_connection(3, "out", 2, "cv");  // Envelope controls modulator level
    patch.add_connection(1, "out", 5, "in");
    patch.add_connection(4, "out", 5, "cv");
    patch.add_connection(5, "out", 100, "in_l");

    patch
}

// ----------------------------------------------------------------------------
// EXPERIMENTAL SOUNDS
// ----------------------------------------------------------------------------

/// Noise Sweep - Filter sweep effect with modulated noise.
pub fn patch_noise_sweep() -> Patch {
    let mut patch = Patch::new("Noise Sweep");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Dramatic filter sweep effect with noise and resonance.".to_string());
    patch.notes = Some(r#"
SIGNAL FLOW:
This patch creates dramatic sweep/riser effects using noise through
a resonant filter, perfect for transitions and buildups.

THE TECHNIQUE:
White noise contains all frequencies equally. When passed through a
resonant bandpass filter, only a narrow band of frequencies passes
through, with the resonance adding a pitched "whistle" at the cutoff.

By modulating the filter cutoff with an LFO, we create sweeping 
motion - the effect sounds like it's rising or falling in pitch.

RESONANCE:
High resonance (0.7) causes the filter to "ring" at its cutoff
frequency, creating a clear pitched element within the noise.
This is key to the dramatic sweep effect.

MODULATION:
The LFO slowly sweeps the filter cutoff up and down. At very slow
rates (0.15 Hz), this creates long, dramatic sweeps. Faster rates
create more rhythmic, pulsing effects.

DISTORTION:
Adds edge and presence, helping the sweep cut through a mix.

TRY: Hold a note and listen to the sweep. Adjust LFO rate for 
different effect speeds. Great for transitions, buildups, and risers.
"#.to_string());
    patch.tags = vec!["effect".into(), "sweep".into(), "noise".into(), "riser".into(), "experimental".into()];

    // OSC - Noise
    patch.add_module(ModuleBuilder::new(1, ModuleType::Oscillator)
        .position(50.0, 50.0)
        .waveform("noise")
        .param_f("level", 0.9)
        .build());

    // Filter - Bandpass with high resonance
    patch.add_module(ModuleBuilder::new(2, ModuleType::Filter)
        .position(250.0, 50.0)
        .filter_mode("bandpass")
        .param_f("cutoff", 1500.0)
        .param_f("resonance", 0.7)
        .build());

    // Sweep LFO
    patch.add_module(ModuleBuilder::new(3, ModuleType::Lfo)
        .position(50.0, 300.0)
        .waveform("triangle")
        .param_f("rate", 0.15)
        .param_f("depth", 0.8)
        .build());

    // Amp Envelope
    patch.add_module(ModuleBuilder::new(4, ModuleType::Envelope)
        .position(250.0, 300.0)
        .param_f("attack", 0.5)
        .param_f("decay", 0.2)
        .param_f("sustain", 0.8)
        .param_f("release", 1.0)
        .build());

    // Amplifier
    patch.add_module(ModuleBuilder::new(5, ModuleType::Amplifier)
        .position(450.0, 50.0)
        .param_f("level", 0.6)
        .build());

    // Distortion
    patch.add_module(ModuleBuilder::new(100, ModuleType::Distortion)
        .position(650.0, 50.0)
        .distortion_mode("soft_clip")
        .param_f("drive", 0.4)
        .param_f("mix", 0.4)
        .build());

    // Reverb
    patch.add_module(ModuleBuilder::new(101, ModuleType::Reverb)
        .position(850.0, 50.0)
        .param_f("room_size", 0.7)
        .param_f("damping", 0.4)
        .param_f("mix", 0.35)
        .build());

    // Connections
    patch.add_connection(1, "out", 2, "in");
    patch.add_connection(2, "out", 5, "in");
    patch.add_connection(3, "out", 2, "cutoff_cv");
    patch.add_connection(4, "out", 5, "cv");
    patch.add_connection(5, "out", 100, "in_l");
    patch.add_connection(100, "out_l", 101, "in_l");

    patch
}
