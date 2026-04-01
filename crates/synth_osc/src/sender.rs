//! OSC sender thread — polls engine state and sends OSC bundles via UDP.
//!
//! Supports an idle mode: when no visualizer responds with `/viz/pong` within
//! `idle_timeout_secs`, the sender skips FFT computation and most messages,
//! only sending a lightweight meta beacon so new clients can discover it.

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use realfft::num_complex::Complex;
use ringbuf::traits::Consumer;
use rosc::{OscBundle, OscMessage, OscPacket, OscTime, OscType, encoder};
use synth_dsp::{FftProcessor, WindowType, fill_window};
use synth_engine::{EngineState, NoteEvent};

use crate::addresses;
use crate::config::OscConfig;

/// Protocol version for `/synth/meta`.
const PROTOCOL_VERSION: i32 = synth_osc_protocol::PROTOCOL_VERSION;

/// FFT size for spectrum analysis.
const FFT_SIZE: usize = 2048;

/// Number of output bands sent via OSC (log-grouped from FFT bins).
const NUM_BANDS: usize = synth_osc_protocol::NUM_FFT_BANDS;

/// Pre-allocated FFT state, reused across ticks.
struct SpectrumState {
    fft: FftProcessor,
    window: Vec<f32>,
    fft_input: Vec<f32>,
    fft_output: Vec<Complex<f32>>,
    bands: [f32; NUM_BANDS],
    /// Previous frame's bands for spectral flux computation.
    prev_bands: [f32; NUM_BANDS],
    /// Pre-computed (lo_bin, hi_bin) ranges for logarithmic band grouping.
    bin_ranges: [(usize, usize); NUM_BANDS],
    /// Spectral centroid in Hz (computed alongside spectrum).
    centroid_hz: f32,
    /// Spectral flux (sum of positive band differences from previous frame).
    flux: f32,
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
            prev_bands: [0.0f32; NUM_BANDS],
            bin_ranges,
            centroid_hz: 0.0,
            flux: 0.0,
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
    status: &Arc<AtomicU8>,
) {
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("OSC: failed to bind UDP socket: {e}");
            return;
        }
    };

    // Don't use connect() — we send to the multicast group address but need
    // to receive unicast /viz/pong replies from individual visualizers.
    // connect() would filter recv() to only the connected address.

    // Non-blocking so we can check for /viz/pong replies
    if let Err(e) = socket.set_nonblocking(true) {
        eprintln!("OSC: failed to set non-blocking: {e}");
        return;
    }

    let target = config.target;

    let tick_interval = Duration::from_secs_f64(1.0 / f64::from(config.update_rate_hz.max(1.0)));
    let meta_interval = Duration::from_secs_f32(config.meta_interval_secs);
    let idle_timeout = Duration::from_secs_f32(config.idle_timeout_secs);

    let mut spectrum = SpectrumState::new();
    let mut messages: Vec<OscPacket> = Vec::with_capacity(16);
    let mut band_args: Vec<OscType> = Vec::with_capacity(NUM_BANDS);
    let mut freq_args: Vec<OscType> = Vec::with_capacity(NUM_BANDS);
    let mut recv_buf = [0u8; 256];

    let mut seq: i32 = 0;
    let mut next_tick = Instant::now();
    let mut next_meta = Instant::now(); // Send meta immediately on start
    let mut last_client_seen = Instant::now() - idle_timeout; // Start in idle mode
    let mut was_idle = true;

    println!(
        "OSC: sending to {} at {:.0} Hz (FFT {FFT_SIZE}-point \u{2192} {NUM_BANDS} bands, idle timeout {:.0}s)",
        config.target, config.update_rate_hz, config.idle_timeout_secs
    );

    while !stop_flag.load(Ordering::Relaxed) {
        let now = Instant::now();

        if now < next_tick {
            // Sleep with small granularity to stay responsive to stop_flag
            let sleep_time = (next_tick - now).min(Duration::from_millis(50));
            std::thread::sleep(sleep_time);
            continue;
        }
        next_tick += tick_interval;

        // Check for /viz/pong replies (non-blocking)
        check_for_pong(&socket, &mut recv_buf, &mut last_client_seen);

        let client_active = now.duration_since(last_client_seen) < idle_timeout;

        if client_active && was_idle {
            println!("OSC: client connected, sending full telemetry");
            status.store(crate::OscStatus::Connected as u8, Ordering::Relaxed);
            was_idle = false;
        } else if !client_active && !was_idle {
            println!("OSC: no client detected, entering idle mode (meta-only)");
            status.store(crate::OscStatus::Idle as u8, Ordering::Relaxed);
            was_idle = true;
        }

        messages.clear();

        // Load sample rate once per tick (used by meta and spectrum)
        let sample_rate = engine_state.sample_rate.load();

        // Sequence number (always first)
        seq = seq.wrapping_add(1);
        messages.push(osc_msg(addresses::META_SEQ, vec![OscType::Int(seq)]));

        // Meta (periodically, always sent even in idle — acts as beacon)
        if now >= next_meta {
            messages.push(osc_msg(
                addresses::META,
                vec![
                    OscType::Int(PROTOCOL_VERSION),
                    OscType::Float(sample_rate as f32),
                    OscType::Float(config.update_rate_hz),
                ],
            ));
            // Include ping so visualizer knows to reply
            messages.push(osc_msg(addresses::VIZ_PING, vec![]));

            // FFT band center frequencies (sent with meta so clients can map bars to Hz)
            freq_args.clear();
            let hz_per_bin = sample_rate as f32 / FFT_SIZE as f32;
            for &(lo_bin, hi_bin) in &spectrum.bin_ranges {
                // Geometric mean of band edges gives the perceptual center frequency
                let center_hz =
                    ((lo_bin as f32 * hz_per_bin) * (hi_bin as f32 * hz_per_bin)).sqrt();
                freq_args.push(OscType::Float(center_hz));
            }
            messages.push(osc_msg(addresses::META_FFT_FREQS, freq_args.clone()));

            next_meta = now + meta_interval;
        }

        // In idle mode, only send meta beacon — skip everything else
        if !client_active {
            // Still drain events to prevent ring buffer overflow
            while event_consumer.try_pop().is_some() {}

            send_bundle(&socket, target, &mut messages);
            continue;
        }

        // === Active mode: full telemetry below ===

        // Drain note events from the ring buffer.
        // Cap per-bundle to avoid exceeding UDP MTU (~1500 bytes).
        const MAX_EVENTS_PER_BUNDLE: usize = 32;
        let mut event_count = 0;
        while let Some(event) = event_consumer.try_pop() {
            event_count += 1;
            if event_count > MAX_EVENTS_PER_BUNDLE {
                // Flush current bundle and start a new one
                send_bundle(&socket, target, &mut messages);
                event_count = 1;
            }
            match event {
                NoteEvent::On {
                    note,
                    velocity,
                    instrument_id,
                    category,
                } => {
                    messages.push(osc_msg(
                        addresses::EVENT_NOTE_ON,
                        vec![
                            OscType::Int(i32::from(note.as_u8())),
                            OscType::Int(i32::from(velocity.to_midi())),
                            OscType::Long(instrument_id.as_u64() as i64),
                            OscType::Int(i32::from(category.as_u8())),
                        ],
                    ));
                }
                NoteEvent::Off {
                    note,
                    instrument_id,
                    category,
                } => {
                    messages.push(osc_msg(
                        addresses::EVENT_NOTE_OFF,
                        vec![
                            OscType::Int(i32::from(note.as_u8())),
                            OscType::Long(instrument_id.as_u64() as i64),
                            OscType::Int(i32::from(category.as_u8())),
                        ],
                    ));
                }
                NoteEvent::Cc { cc, value, channel } => {
                    messages.push(osc_msg(
                        addresses::EVENT_CC,
                        vec![
                            OscType::Int(i32::from(cc.as_u8())),
                            OscType::Float(value.as_f32()),
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
        compute_spectrum(
            &engine_state.master_scope,
            &mut spectrum,
            sample_rate as f32,
        );
        band_args.clear();
        band_args.extend(spectrum.bands.iter().map(|&v| OscType::Float(v)));
        messages.push(osc_msg(addresses::AUDIO_FFT, band_args.clone()));

        // Spectral centroid (brightness)
        messages.push(osc_msg(
            addresses::AUDIO_CENTROID,
            vec![OscType::Float(spectrum.centroid_hz)],
        ));

        // Spectral flux (onset/change detection)
        messages.push(osc_msg(
            addresses::AUDIO_FLUX,
            vec![OscType::Float(spectrum.flux)],
        ));

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

        // Beat phase (fractional part of beat position, 0.0–1.0)
        let beat_phase = beats.fract() as f32;
        messages.push(osc_msg(
            addresses::TRANSPORT_PHASE,
            vec![OscType::Float(beat_phase)],
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

        // Event ring buffer drops (swap to zero atomically)
        let drops = engine_state
            .event_drops
            .swap(0, std::sync::atomic::Ordering::Relaxed);
        if drops > 0 {
            messages.push(osc_msg(
                addresses::ENGINE_EVENT_DROPS,
                vec![OscType::Int(drops as i32)],
            ));
        }

        send_bundle(&socket, target, &mut messages);
    }

    println!("OSC: sender stopped");
}

/// Check for `/viz/pong` replies on the socket (non-blocking).
fn check_for_pong(socket: &UdpSocket, buf: &mut [u8], last_client_seen: &mut Instant) {
    while let Ok(size) = socket.recv(buf) {
        if let Ok((_, packet)) = rosc::decoder::decode_udp(&buf[..size])
            && contains_pong(&packet)
        {
            *last_client_seen = Instant::now();
        }
    }
}

/// Check if an OSC packet contains a `/viz/pong` message.
fn contains_pong(packet: &OscPacket) -> bool {
    match packet {
        OscPacket::Message(msg) => msg.addr == addresses::VIZ_PONG,
        OscPacket::Bundle(bundle) => bundle.content.iter().any(contains_pong),
    }
}

/// Encode and send an OSC bundle to the target address, consuming the messages vec.
fn send_bundle(socket: &UdpSocket, target: SocketAddr, messages: &mut Vec<OscPacket>) {
    let bundle = OscBundle {
        timetag: OscTime {
            seconds: 0,
            fractional: 1,
        },
        content: std::mem::take(messages),
    };

    match encoder::encode(&OscPacket::Bundle(bundle)) {
        Ok(bytes) => {
            if let Err(e) = socket.send_to(&bytes, target) {
                eprintln!("OSC send error to {target}: {e}");
            }
        }
        Err(e) => {
            eprintln!("OSC encode error: {e}");
        }
    }
}

/// Compute FFT spectrum from the master scope snapshot and reduce to `NUM_BANDS` bands.
///
/// Copies the left-channel snapshot directly into the pre-allocated FFT input buffer
/// (with windowing applied), runs a forward FFT, and groups bins logarithmically
/// using pre-computed bin ranges. Output bands are normalized to 0.0–1.0.
///
/// Also computes spectral centroid (Hz) and spectral flux (positive difference
/// from previous frame).
fn compute_spectrum(
    master_scope: &synth_engine::visualizers::VisualizationBuffer,
    state: &mut SpectrumState,
    sample_rate: f32,
) {
    // Copy snapshot data directly into fft_input under the lock, with windowing.
    master_scope.copy_snapshot_windowed_into(&mut state.fft_input, &state.window);

    // Forward FFT
    state
        .fft
        .forward(&mut state.fft_input, &mut state.fft_output);

    // Save previous bands for flux computation
    state.prev_bands = state.bands;

    // Group bins logarithmically into NUM_BANDS using pre-computed ranges.
    state.bands.fill(0.0);

    // Also accumulate for spectral centroid (weighted average frequency)
    let hz_per_bin = sample_rate / FFT_SIZE as f32;
    let mut weighted_sum = 0.0f32;
    let mut magnitude_sum = 0.0f32;

    for (band_idx, &(lo_bin, hi_bin)) in state.bin_ranges.iter().enumerate() {
        let mut sum = 0.0f32;
        let mut count = 0u32;

        for bin in lo_bin..hi_bin {
            let mag = state.fft_output[bin].norm();
            sum += mag;
            count += 1;

            // Accumulate for centroid
            let freq = bin as f32 * hz_per_bin;
            weighted_sum += freq * mag;
            magnitude_sum += mag;
        }

        if count > 0 {
            let avg_mag = sum / count as f32;
            // Convert to dB and normalize to 0.0–1.0 range (-100 dB → 0 dB)
            let db = 20.0 * (avg_mag / FFT_SIZE as f32).max(1e-10).log10();
            state.bands[band_idx] = ((db + 100.0) / 100.0).clamp(0.0, 1.0);
        }
    }

    // Spectral centroid
    state.centroid_hz = if magnitude_sum > 1e-10 {
        weighted_sum / magnitude_sum
    } else {
        0.0
    };

    // Spectral flux (sum of positive differences between current and previous bands)
    state.flux = state
        .bands
        .iter()
        .zip(state.prev_bands.iter())
        .map(|(&curr, &prev)| (curr - prev).max(0.0))
        .sum();
}

/// Helper to build an OSC message.
fn osc_msg(addr: &str, args: Vec<OscType>) -> OscPacket {
    OscPacket::Message(OscMessage {
        addr: addr.to_owned(),
        args,
    })
}
