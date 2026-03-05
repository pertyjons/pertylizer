//! OSC sender thread — polls engine state and sends OSC bundles via UDP.

use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use realfft::num_complex::Complex;
use ringbuf::traits::Consumer;
use rosc::{OscBundle, OscMessage, OscPacket, OscTime, OscType, encoder};
use synth_dsp::{FftProcessor, WindowType, fill_window};
use synth_engine::{EngineState, NoteEvent};

use crate::addresses;
use crate::config::OscConfig;

/// Protocol version for `/synth/meta`.
const PROTOCOL_VERSION: i32 = 1;

/// FFT size for spectrum analysis.
const FFT_SIZE: usize = 2048;

/// Number of output bands sent via OSC (log-grouped from FFT bins).
const NUM_BANDS: usize = 128;

/// Pre-allocated FFT state, reused across ticks.
struct SpectrumState {
    fft: FftProcessor,
    window: Vec<f32>,
    fft_input: Vec<f32>,
    fft_output: Vec<Complex<f32>>,
    bands: [f32; NUM_BANDS],
    /// Pre-computed (lo_bin, hi_bin) ranges for logarithmic band grouping.
    bin_ranges: [(usize, usize); NUM_BANDS],
}

impl SpectrumState {
    fn new() -> Self {
        let complex_size = FFT_SIZE / 2 + 1;
        let num_bins = complex_size - 1; // exclude DC

        let mut window = vec![0.0f32; FFT_SIZE];
        fill_window(&mut window, WindowType::Hann);

        // Pre-compute logarithmic bin ranges once.
        let mut bin_ranges = [(0usize, 0usize); NUM_BANDS];
        for band_idx in 0..NUM_BANDS {
            let lo_frac = (band_idx as f64) / (NUM_BANDS as f64);
            let hi_frac = ((band_idx + 1) as f64) / (NUM_BANDS as f64);

            let lo_bin = ((num_bins as f64).powf(lo_frac) as usize).max(1);
            let hi_bin = ((num_bins as f64).powf(hi_frac) as usize)
                .max(lo_bin + 1)
                .min(complex_size);

            bin_ranges[band_idx] = (lo_bin, hi_bin);
        }

        Self {
            fft: FftProcessor::new(FFT_SIZE),
            window,
            fft_input: vec![0.0f32; FFT_SIZE],
            fft_output: vec![Complex::new(0.0f32, 0.0); complex_size],
            bands: [0.0f32; NUM_BANDS],
            bin_ranges,
        }
    }
}

/// Run the OSC sender loop. Blocks until `stop_flag` is set.
#[allow(clippy::too_many_lines)]
pub(crate) fn run(
    config: &OscConfig,
    engine_state: &Arc<EngineState>,
    mut event_consumer: ringbuf::HeapCons<NoteEvent>,
    stop_flag: &Arc<AtomicBool>,
) {
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("OSC: failed to bind UDP socket: {e}");
            return;
        }
    };

    if let Err(e) = socket.connect(config.target) {
        eprintln!("OSC: failed to connect to {}: {e}", config.target);
        return;
    }

    let tick_interval = Duration::from_secs_f64(1.0 / f64::from(config.update_rate_hz));
    let meta_interval = Duration::from_secs_f32(config.meta_interval_secs);

    let mut spectrum = SpectrumState::new();
    let mut messages: Vec<OscPacket> = Vec::with_capacity(16);
    let mut band_args: Vec<OscType> = Vec::with_capacity(NUM_BANDS);

    let mut seq: i32 = 0;
    let mut next_tick = Instant::now();
    let mut next_meta = Instant::now(); // Send meta immediately on start

    println!(
        "OSC: sending to {} at {:.0} Hz (FFT {FFT_SIZE}-point → {NUM_BANDS} bands)",
        config.target, config.update_rate_hz
    );

    while !stop_flag.load(Ordering::Relaxed) {
        let now = Instant::now();

        if now < next_tick {
            std::thread::sleep(next_tick - now);
        }
        next_tick += tick_interval;

        messages.clear();

        // Sequence number (always first)
        seq = seq.wrapping_add(1);
        messages.push(osc_msg(addresses::META_SEQ, vec![OscType::Int(seq)]));

        // Meta (periodically)
        if now >= next_meta {
            let sample_rate = engine_state.sample_rate.load();
            messages.push(osc_msg(
                addresses::META,
                vec![
                    OscType::Int(PROTOCOL_VERSION),
                    OscType::Float(sample_rate as f32),
                    OscType::Float(config.update_rate_hz),
                ],
            ));
            next_meta = now + meta_interval;
        }

        // Drain note events from the ring buffer
        while let Some(event) = event_consumer.try_pop() {
            match event {
                NoteEvent::On {
                    note,
                    velocity,
                    channel,
                } => {
                    messages.push(osc_msg(
                        addresses::EVENT_NOTE_ON,
                        vec![
                            OscType::Int(i32::from(note.as_u8())),
                            OscType::Int(i32::from(velocity.to_midi())),
                            OscType::Int(i32::from(channel.as_zero_indexed())),
                        ],
                    ));
                }
                NoteEvent::Off { note, channel } => {
                    messages.push(osc_msg(
                        addresses::EVENT_NOTE_OFF,
                        vec![
                            OscType::Int(i32::from(note.as_u8())),
                            OscType::Int(i32::from(channel.as_zero_indexed())),
                        ],
                    ));
                }
            }
        }

        // RMS levels
        let (rms_l, rms_r) = engine_state.meters.get_rms();
        messages.push(osc_msg(
            addresses::AUDIO_RMS,
            vec![
                OscType::Float(rms_l.as_f32()),
                OscType::Float(rms_r.as_f32()),
            ],
        ));

        // Peak levels
        let (peak_l, peak_r) = engine_state.meters.get_peak();
        messages.push(osc_msg(
            addresses::AUDIO_PEAK,
            vec![
                OscType::Float(peak_l.as_f32()),
                OscType::Float(peak_r.as_f32()),
            ],
        ));

        // FFT spectrum from master scope snapshot
        compute_spectrum(&engine_state.master_scope, &mut spectrum);
        band_args.clear();
        band_args.extend(spectrum.bands.iter().map(|&v| OscType::Float(v)));
        messages.push(osc_msg(addresses::AUDIO_FFT, band_args.clone()));

        // Transport state
        let playing = engine_state.transport.is_playing();
        let tempo = engine_state.transport.get_tempo();
        let beats = engine_state.transport.position_beats.load();
        messages.push(osc_msg(
            addresses::TRANSPORT_STATE,
            vec![
                OscType::Int(i32::from(playing)),
                OscType::Float(tempo.as_f32()),
                OscType::Float(beats as f32),
            ],
        ));

        // Voice count
        let voices = engine_state.voice_count.load();
        messages.push(osc_msg(
            addresses::ENGINE_VOICE_COUNT,
            vec![OscType::Int(voices as i32)],
        ));

        // CPU usage
        let cpu = engine_state.cpu_usage.load();
        messages.push(osc_msg(
            addresses::ENGINE_CPU,
            vec![OscType::Float(cpu * 100.0)],
        ));

        // Encode and send bundle
        let bundle = OscBundle {
            timetag: OscTime {
                seconds: 0,
                fractional: 1,
            },
            content: std::mem::take(&mut messages),
        };

        if let Ok(bytes) = encoder::encode(&OscPacket::Bundle(bundle)) {
            let _ = socket.send(&bytes);
        }
    }

    println!("OSC: sender stopped");
}

/// Compute FFT spectrum from the master scope snapshot and reduce to `NUM_BANDS` bands.
///
/// Copies the left-channel snapshot directly into the pre-allocated FFT input buffer
/// (with windowing applied), runs a forward FFT, and groups bins logarithmically
/// using pre-computed bin ranges. Output bands are normalized to 0.0–1.0.
fn compute_spectrum(
    master_scope: &synth_engine::visualizers::VisualizationBuffer,
    state: &mut SpectrumState,
) {
    // Copy snapshot data directly into fft_input under the lock, with windowing.
    // This avoids allocating a Vec clone.
    master_scope.copy_snapshot_windowed_into(&mut state.fft_input, &state.window);

    // Forward FFT
    state
        .fft
        .forward(&mut state.fft_input, &mut state.fft_output);

    // Group bins logarithmically into NUM_BANDS using pre-computed ranges.
    state.bands.fill(0.0);

    for (band_idx, &(lo_bin, hi_bin)) in state.bin_ranges.iter().enumerate() {
        let mut sum = 0.0f32;
        let mut count = 0u32;

        for bin in lo_bin..hi_bin {
            sum += state.fft_output[bin].norm();
            count += 1;
        }

        if count > 0 {
            let avg_mag = sum / count as f32;
            // Convert to dB and normalize to 0.0–1.0 range (-100 dB → 0 dB)
            let db = 20.0 * (avg_mag / FFT_SIZE as f32).max(1e-10).log10();
            state.bands[band_idx] = ((db + 100.0) / 100.0).clamp(0.0, 1.0);
        }
    }
}

/// Helper to build an OSC message.
fn osc_msg(addr: &str, args: Vec<OscType>) -> OscPacket {
    OscPacket::Message(OscMessage {
        addr: addr.to_owned(),
        args,
    })
}
