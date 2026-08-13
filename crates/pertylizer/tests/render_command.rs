//! Integration tests for the one-shot `pertylizer render` command
//! (`pertylizer::render::run_render_command`).
//!
//! Each test writes a synthetic project to a temp dir, renders it through the
//! same library entry point the subcommand calls, and inspects the WAV and the
//! JSON receipt. Nothing here opens an audio device or a window, which is the
//! point: the command has to work on a CI box with neither.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use synth_core::{InstrumentId, Seconds};
use synth_sequencer::{
    Duration as SeqDuration, PatternTick, Pitch, SharedSong, Song, Tick, TrackId, Velocity,
};

use pertylizer::project_apply::{ProjectBuildOptions, save_project_to};
use pertylizer::render::{
    MAX_RENDER_BYTES, MAX_RENDER_SAMPLE_RATE, MAX_TAIL_SECONDS, MixSelection, RenderCommand,
    RenderError, TrackSelector, WavFormat, run_render_command,
};

use common::{TEST_SR, setup_with_patch, sustain_patch};

/// Two tracks on one instrument, an octave apart, so soloing either produces
/// audibly — and byte-wise — different output.
const LOW_TRACK: TrackId = TrackId(0);
const HIGH_TRACK: TrackId = TrackId(1);

/// Build the two-track song the fixtures render.
fn two_track_song() -> Arc<SharedSong> {
    let mut song = Song::new("RenderCommandFixture");
    for (index, root) in [48_u8, 72].iter().enumerate() {
        let pattern_id = song.create_pattern(SeqDuration::WHOLE);
        if let Some(pattern) = song.pattern_mut(pattern_id) {
            for step in 0..4_u32 {
                let midi = root + u8::try_from(step).unwrap_or(0);
                let note = pattern.add_note(
                    PatternTick(step * 960),
                    Pitch::new(midi).expect("valid MIDI note"),
                    Velocity::MF,
                );
                if let Some(note) = pattern.note_mut(note) {
                    note.duration = Some(SeqDuration(900));
                }
            }
        }
        let track_id = song.create_track(if index == 0 { "Low" } else { "High" });
        if let Some(track) = song.track_mut(track_id) {
            track.instrument = InstrumentId::new(0);
        }
        assert!(song.place_pattern(pattern_id, track_id, Tick(0)));
    }
    assert_eq!(song.track(LOW_TRACK).map(|t| t.name.as_str()), Some("Low"));
    assert_eq!(
        song.track(HIGH_TRACK).map(|t| t.name.as_str()),
        Some("High")
    );
    Arc::new(SharedSong::new(song))
}

/// Save the fixture project to `path`, optionally with a mix state already
/// stored in the file.
fn write_project(path: &Path, prepare: impl FnOnce(&mut Song)) {
    let rig = setup_with_patch(&sustain_patch());
    let song = two_track_song();
    prepare(&mut song.write());
    save_project_to(
        path,
        &rig.session,
        &song,
        &rig.sample_library,
        ProjectBuildOptions::default(),
    )
    .expect("saving the fixture project should succeed");
}

/// A render command with the fixture defaults: half a second, no tail, no mix
/// flags, receipt next to the WAV.
fn command(input: &Path, output: PathBuf) -> RenderCommand {
    // Appended, not `with_extension("json")`: that turns `soloed.wav` into
    // `soloed.json`, which for a fixture whose input is `soloed.json` is the
    // input file — and the command rightly refuses to write over it.
    let mut result_json = output.clone().into_os_string();
    result_json.push(".json");
    let result_json = PathBuf::from(result_json);
    RenderCommand {
        input: input.to_path_buf(),
        output,
        sample_rate: TEST_SR,
        format: WavFormat::default(),
        seconds: Seconds::new(0.5),
        tail: Seconds::ZERO,
        result_json: Some(result_json),
        mix: MixSelection::default(),
        argv: vec!["pertylizer".to_string(), "render".to_string()],
    }
}

fn selector(raw: &str) -> TrackSelector {
    raw.parse().unwrap_or_else(|_| unreachable!())
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The rendered file has to be a real float WAV whose length agrees with what
/// the receipt claims — a receipt that describes a different file than the one
/// on disk is worse than no receipt.
#[test]
fn the_wav_parses_and_matches_the_receipt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("fixture.json");
    write_project(&input, |_| {});

    let output = dir.path().join("render.wav");
    let request = command(&input, output.clone());
    let receipt = run_render_command(&request).expect("render succeeds");

    let reader = hound::WavReader::open(&output).expect("output should be a readable WAV");
    let spec = reader.spec();
    assert_eq!(spec.sample_rate, TEST_SR);
    assert_eq!(spec.bits_per_sample, 32);
    assert_eq!(spec.sample_format, hound::SampleFormat::Float);
    assert_eq!(spec.channels, receipt.audio.channels);

    let frames = u64::from(reader.len()) / u64::from(spec.channels);
    assert_eq!(frames, receipt.audio.frames);
    assert_eq!(receipt.output.bytes, read(&output).len() as u64);
    assert!(
        receipt.audio.peak > 0.0,
        "the fixture should not render silence"
    );
    assert!(receipt.warnings.is_empty(), "{:?}", receipt.warnings);
    assert!(
        !receipt.load_summary.is_empty(),
        "the receipt must record what the load reported"
    );

    // The receipt on disk is the receipt that was returned.
    let written: serde_json::Value =
        serde_json::from_slice(&read(&receipt_path(&request))).expect("the receipt is valid JSON");
    assert_eq!(written["output"]["sha256"], receipt.output.sha256);
    assert_eq!(written["protocol_version"], receipt.protocol_version);
    assert_eq!(written["audio"]["frames"], receipt.audio.frames);
    assert_eq!(written["audio"]["bit_depth"], 32);
    assert_eq!(written["audio"]["sample_format"], "float");
}

/// Every `--bit-depth` must produce a WAV whose header says what was asked for,
/// with the receipt agreeing. The header is the part a consumer reads to decide
/// how to decode the file, so a format that rendered but mislabelled itself
/// would be worse than one that failed.
#[test]
fn every_bit_depth_writes_a_matching_header_and_receipt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("fixture.json");
    write_project(&input, |_| {});

    for format in WavFormat::ALL {
        let output = dir.path().join(format!("render-{}.wav", format.label()));
        let mut request = command(&input, output.clone());
        request.format = format;
        let receipt = run_render_command(&request)
            .unwrap_or_else(|e| panic!("{} should render: {e}", format.label()));

        let reader = hound::WavReader::open(&output)
            .unwrap_or_else(|e| panic!("{} should be readable: {e}", format.label()));
        let spec = reader.spec();
        assert_eq!(
            spec.bits_per_sample,
            format.bits_per_sample(),
            "{}",
            format.label()
        );
        let expected_encoding = if format == WavFormat::Float32 {
            hound::SampleFormat::Float
        } else {
            hound::SampleFormat::Int
        };
        assert_eq!(spec.sample_format, expected_encoding, "{}", format.label());

        assert_eq!(
            receipt.audio.bit_depth,
            format.bits_per_sample(),
            "{}",
            format.label()
        );
        assert_eq!(receipt.audio.frames, u64::from(reader.len()) / 2);
        assert_eq!(receipt.output.bytes, read(&output).len() as u64);
    }
}

/// A narrower format writes a proportionally smaller file at the same frame
/// count. This is the property a `--bit-depth` flag exists for, and it also
/// catches a format that silently fell back to the 32-bit float default.
#[test]
fn a_narrower_format_writes_a_smaller_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("fixture.json");
    write_project(&input, |_| {});

    let mut sizes = Vec::new();
    for format in [WavFormat::Int8, WavFormat::Int16, WavFormat::Float32] {
        let output = dir.path().join(format!("size-{}.wav", format.label()));
        let mut request = command(&input, output.clone());
        request.format = format;
        let receipt = run_render_command(&request).expect("render succeeds");
        sizes.push((format, receipt.audio.frames, receipt.output.bytes));
    }

    // Same window, same rate: the frame count must not depend on the format.
    let frames = sizes[0].1;
    assert!(
        sizes.iter().all(|(_, f, _)| *f == frames),
        "frame counts differ across formats: {sizes:?}"
    );
    assert!(
        sizes[0].2 < sizes[1].2 && sizes[1].2 < sizes[2].2,
        "8-bit must be smaller than 16-bit, which must be smaller than 32-bit float: {sizes:?}"
    );
}

/// Integer output clamps everything past ±1.0, so the receipt warns when the
/// engine's peak went over. The fixture renders well under full scale, so no
/// format may claim it clipped — a warning that fired on every integer render
/// would be noise a harness learns to ignore, which is how the real overshoot
/// gets missed.
#[test]
fn an_unclipped_integer_render_does_not_warn_about_clipping() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("fixture.json");
    write_project(&input, |_| {});

    for format in WavFormat::ALL {
        let output = dir.path().join(format!("clean-{}.wav", format.label()));
        let mut request = command(&input, output);
        request.format = format;
        let receipt = run_render_command(&request).expect("render succeeds");
        assert!(
            receipt.audio.peak <= 1.0,
            "{} fixture should render below full scale, got {}",
            format.label(),
            receipt.audio.peak
        );
        assert!(
            !receipt.warnings.iter().any(|w| w.contains("clipped")),
            "{} must not warn about clipping: {:?}",
            format.label(),
            receipt.warnings
        );
    }
}

/// The render buffer is `f32` whatever the output format is, so validation must
/// reach the same verdict for a narrow format as for a wide one. Otherwise
/// `--bit-depth 8` would let through a request larger than the buffer the
/// renderer can actually allocate.
///
/// **This test was weakened when `MAX_RENDER_SAMPLE_RATE` became the engine
/// ceiling.** It used to drive both formats into `RenderTooLarge` at 384 kHz,
/// which proved the guard measured the `f32` render buffer rather than the
/// output format. No legal request can reach that budget any more — see
/// `the_other_bounds_cannot_reach_the_size_budget` — so what remains testable
/// is that the two formats agree, at the largest request the bounds allow. A
/// guard that grew format-dependent in the permissive direction would no longer
/// be caught here; one that grew format-dependent in the restrictive direction
/// still would.
#[test]
fn validation_does_not_depend_on_the_output_format() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("fixture.json");
    write_project(&input, |_| {});

    let mut request = command(&input, dir.path().join("wide.wav"));
    request.sample_rate = MAX_RENDER_SAMPLE_RATE;
    request.seconds = Seconds::new(1.0);

    for format in [WavFormat::Float32, WavFormat::Int8] {
        request.format = format;
        assert!(
            !matches!(
                run_render_command(&request),
                Err(RenderError::RenderTooLarge { .. })
            ),
            "{} must reach the same size verdict",
            format.label()
        );
    }
}

/// `LIMIT-0004`: the render command used to accept up to 384 kHz while the
/// engine ceiling is 192 kHz, and nothing rejected the difference. A render
/// above the ceiling silently got less DSP than it asked for — the limiter's
/// look-ahead ring is sized from the ceiling, so an advertised 5 ms became
/// 2.5 ms.
#[test]
fn a_sample_rate_above_the_engine_ceiling_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("fixture.json");
    write_project(&input, |_| {});

    let mut request = command(&input, dir.path().join("too-fast.wav"));
    request.sample_rate = 384_000;
    request.seconds = Seconds::new(1.0);

    assert!(matches!(
        run_render_command(&request),
        Err(RenderError::InvalidSampleRate(384_000))
    ));
}

/// The two ceilings must not desync again: the render command's is *derived*
/// from the engine's, and this pins that it stays derived rather than being
/// hand-copied back.
#[test]
fn the_render_ceiling_is_the_engine_ceiling() {
    assert_eq!(
        MAX_RENDER_SAMPLE_RATE,
        synth_core::audio::DeviceSampleRate::MAX_SUPPORTED.as_u32()
    );
}

/// `MAX_RENDER_BYTES` is a backstop, not a reachable check: the duration, tail,
/// and rate bounds together cap one render below it. That is a relationship
/// between four independent constants, so it is pinned rather than assumed —
/// raising any of them fails here, which is the moment to re-check whether the
/// allocation guard is armed again.
#[test]
fn the_other_bounds_cannot_reach_the_size_budget() {
    let largest =
        f64::from(pertylizer::audio::arrangement_render::MAX_RENDER_SECONDS + MAX_TAIL_SECONDS)
            * f64::from(MAX_RENDER_SAMPLE_RATE)
            * 2.0
            * 4.0;
    assert!(
        largest <= MAX_RENDER_BYTES as f64,
        "the bounds now reach {largest} bytes against a {MAX_RENDER_BYTES}-byte budget: \
         the size guard is reachable again, so `validation_does_not_depend_on_the_output_format` \
         should go back to driving both formats into `RenderTooLarge`"
    );
}

fn receipt_path(command: &RenderCommand) -> PathBuf {
    command
        .result_json
        .clone()
        .expect("the fixture command always writes a receipt")
}

/// The whole point of the contract: the same command on the same input has to
/// produce the same bytes, or an A/B budget measured against it means nothing.
#[test]
fn repeated_renders_are_byte_identical() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("fixture.json");
    write_project(&input, |_| {});

    let first = run_render_command(&command(&input, dir.path().join("a.wav"))).expect("first");
    let second = run_render_command(&command(&input, dir.path().join("b.wav"))).expect("second");

    assert_eq!(first.output.sha256, second.output.sha256);
    assert_eq!(
        read(&dir.path().join("a.wav")),
        read(&dir.path().join("b.wav"))
    );
}

/// The MCP tool and the command must be the same render, not two renders that
/// happen to agree. Both are driven at the same scope here so any divergence is
/// a divergence in the shared path.
#[test]
fn the_mcp_tool_renders_the_same_bytes() {
    use pertylizer::mcp_bridge::render_to_wav_with_tail_impl;
    use pertylizer::mcp_shared::McpSharedState;

    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("fixture.json");
    write_project(&input, |_| {});

    let via_command = dir.path().join("command.wav");
    run_render_command(&command(&input, via_command.clone())).expect("command render");

    let project = pertylizer::render::headless::load_project_file(&input).expect("load");
    let shared = McpSharedState::with_song(Arc::clone(&project.song));
    let via_mcp = dir.path().join("mcp.wav");
    render_to_wav_with_tail_impl(
        &project.session,
        &project.sample_library,
        &shared,
        via_mcp.to_string_lossy().into_owned(),
        0.5,
        Some(Tick(0)),
        None,
        // The command always renders the full chain; match it, or the two would
        // differ for the uninteresting reason that they were asked for
        // different signal paths.
        synth_core::AnalysisScope {
            master_effects: true,
            return_effects: true,
            render_sample_rate: synth_core::audio::DeviceSampleRate::new(TEST_SR),
        },
        Seconds::ZERO,
    )
    .expect("mcp render");

    assert_eq!(read(&via_command), read(&via_mcp));
}

/// A tail is extra audio *after* the window, so it can only make the file
/// longer — and the receipt has to say by how much.
#[test]
fn a_tail_lengthens_the_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("fixture.json");
    write_project(&input, |_| {});

    let dry = run_render_command(&command(&input, dir.path().join("dry.wav"))).expect("dry");

    let mut with_tail = command(&input, dir.path().join("tail.wav"));
    with_tail.tail = Seconds::new(0.25);
    let wet = run_render_command(&with_tail).expect("tail");

    assert_eq!(dry.audio.tail_seconds, 0.0);
    assert_eq!(wet.audio.tail_seconds, 0.25);
    assert_eq!(dry.audio.end_tick, wet.audio.end_tick, "same window");
    let extra = wet.audio.frames - dry.audio.frames;
    let expected = u64::from(TEST_SR) / 4;
    assert!(
        extra.abs_diff(expected) <= 1,
        "expected ~{expected} extra frames, got {extra}"
    );
}

/// Selecting a track by id and by its (unique) name has to mean the same
/// track — otherwise the convenience form is a trap.
#[test]
fn solo_by_id_and_by_name_agree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("fixture.json");
    write_project(&input, |_| {});

    let mut by_id = command(&input, dir.path().join("by-id.wav"));
    by_id.mix.solo = vec![selector("1")];
    let id_receipt = run_render_command(&by_id).expect("solo by id");

    let mut by_name = command(&input, dir.path().join("by-name.wav"));
    by_name.mix.solo = vec![selector("High")];
    let name_receipt = run_render_command(&by_name).expect("solo by name");

    assert_eq!(id_receipt.mix.soloed, vec![HIGH_TRACK.0]);
    assert_eq!(name_receipt.mix.soloed, vec![HIGH_TRACK.0]);
    assert_eq!(id_receipt.output.sha256, name_receipt.output.sha256);

    // And soloing the *other* track is a different render, so the flag is
    // actually reaching the audio rather than being recorded and ignored.
    let mut low = command(&input, dir.path().join("low.wav"));
    low.mix.solo = vec![selector("Low")];
    let low_receipt = run_render_command(&low).expect("solo low");
    assert_ne!(low_receipt.output.sha256, id_receipt.output.sha256);
}

/// Muting is the complement of soloing over this two-track fixture, and muting
/// everything leaves nothing to hear.
#[test]
fn mute_removes_exactly_the_named_track() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("fixture.json");
    write_project(&input, |_| {});

    let mut mute_low = command(&input, dir.path().join("mute-low.wav"));
    mute_low.mix.mute = vec![selector("Low")];
    let muted = run_render_command(&mute_low).expect("mute low");
    assert_eq!(muted.mix.muted, vec![LOW_TRACK.0]);
    assert_eq!(
        muted.mix.audible.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![HIGH_TRACK.0]
    );

    let mut solo_high = command(&input, dir.path().join("solo-high.wav"));
    solo_high.mix.solo = vec![selector("High")];
    let soloed = run_render_command(&solo_high).expect("solo high");
    assert_eq!(muted.output.sha256, soloed.output.sha256);

    let mut mute_all = command(&input, dir.path().join("silent.wav"));
    mute_all.mix.mute = vec![selector("Low"), selector("High")];
    let silent = run_render_command(&mute_all).expect("mute everything");
    assert!(silent.mix.audible.is_empty());
    assert_eq!(silent.audio.peak, 0.0, "muting every track must be silent");
    // A valid WAV of nothing exits zero like any other render, so the receipt
    // has to say so. The warning keys off the audible set, not off `peak` — a
    // project with master effects returns a small residual rather than exactly
    // zero even with every track muted.
    assert!(
        silent.warnings.iter().any(|w| w.contains("no track")),
        "a render with nothing audible must say so: {:?}",
        silent.warnings
    );
}

/// Naming one track in both flags looks like "mute wins", but the setters are
/// mutually exclusive: muting the only soloed track clears the solo and the
/// render becomes the *complement* — every other track. Refused, and refused
/// before anything is written.
#[test]
fn soloing_and_muting_one_track_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("fixture.json");
    write_project(&input, |_| {});

    let output = dir.path().join("both.wav");
    let mut both = command(&input, output.clone());
    both.mix.solo = vec![selector("High")];
    both.mix.mute = vec![selector("High")];

    let error = run_render_command(&both).expect_err("solo and mute of one track");
    assert!(matches!(error, RenderError::MixSelection(_)), "{error:?}");
    assert!(!output.exists(), "nothing may be written");
}

/// A file saved with a solo must not render as if that solo were a command-line
/// flag — and because the override is otherwise invisible, it has to be said
/// out loud in the receipt.
#[test]
fn a_saved_solo_is_overridden_and_reported() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("soloed.json");
    write_project(&input, |song| {
        if let Some(track) = song.track_mut(HIGH_TRACK) {
            track.set_solo(true);
        }
    });

    let plain = dir.path().join("full-mix.wav");
    let receipt = run_render_command(&command(&input, plain)).expect("render");

    assert_eq!(
        receipt.mix.audible.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![LOW_TRACK.0, HIGH_TRACK.0],
        "no flags means the full mix, whatever the file saved"
    );
    assert!(
        receipt
            .warnings
            .iter()
            .any(|w| w.contains("1 soloed") && w.contains("cleared")),
        "the override must be reported: {:?}",
        receipt.warnings
    );

    // The same project rendered with an explicit solo differs, proving the
    // full-mix render above really was the full mix.
    let mut soloed = command(&input, dir.path().join("soloed.wav"));
    soloed.mix.solo = vec![selector("High")];
    let soloed = run_render_command(&soloed).expect("render");
    assert_ne!(receipt.output.sha256, soloed.output.sha256);
}

/// A selection that cannot resolve has to fail before the render, leaving no
/// half-written WAV and no receipt describing a file that does not exist.
#[test]
fn an_unresolvable_selection_leaves_no_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("fixture.json");
    write_project(&input, |_| {});

    for (label, raw) in [("unknown id", "42"), ("unknown name", "Strings")] {
        let output = dir.path().join(format!("{label}.wav"));
        let mut bad = command(&input, output.clone());
        bad.mix.solo = vec![selector(raw)];
        let error = run_render_command(&bad).expect_err(label);
        assert!(
            matches!(error, RenderError::MixSelection(_)),
            "{label}: {error:?}"
        );
        assert!(!output.exists(), "{label} wrote a WAV anyway");
        assert!(
            !receipt_path(&bad).exists(),
            "{label} wrote a receipt anyway"
        );
    }
}

/// Two tracks share a name here, so the name does not identify one of them and
/// the command must say so rather than pick.
#[test]
fn an_ambiguous_name_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("twins.json");
    write_project(&input, |song| {
        if let Some(track) = song.track_mut(HIGH_TRACK) {
            track.name = "Low".to_string();
        }
    });

    let output = dir.path().join("ambiguous.wav");
    let mut bad = command(&input, output.clone());
    bad.mix.solo = vec![selector("Low")];
    let error = run_render_command(&bad).expect_err("two tracks named Low");
    assert!(error.to_string().contains("2 tracks"), "{error}");
    assert!(!output.exists());
}

/// An input that is not a project must fail as a load, and must not leave
/// anything behind either.
#[test]
fn an_unloadable_input_leaves_no_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("garbage.json");
    std::fs::write(&input, b"{\"not\": \"a project\"}").expect("write");

    let output = dir.path().join("garbage.wav");
    let error =
        run_render_command(&command(&input, output.clone())).expect_err("garbage is not a project");
    assert!(
        matches!(error, RenderError::ProjectLoad { .. }),
        "{error:?}"
    );
    assert!(!output.exists());
}

/// The bundle path dispatches on the ZIP magic bytes rather than the
/// extension, so a truncated bundle reaches the archive reader and has to come
/// back as a typed load failure rather than a panic or a silent empty render.
#[test]
fn a_corrupt_bundle_leaves_no_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("truncated.ptzb");
    // A ZIP local-file-header signature and nothing that follows it.
    std::fs::write(&input, b"PK\x03\x04truncated").expect("write");

    let output = dir.path().join("bundle.wav");
    let error =
        run_render_command(&command(&input, output.clone())).expect_err("a truncated bundle");
    assert!(
        matches!(error, RenderError::ProjectLoad { .. }),
        "{error:?}"
    );
    assert!(!output.exists());
}

/// The command renders from an in-memory copy of the mix state. It must never
/// write the input back, however much it changed.
#[test]
fn the_input_file_is_never_written() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("fixture.json");
    write_project(&input, |_| {});
    let before = read(&input);

    let mut soloed = command(&input, dir.path().join("out.wav"));
    soloed.mix.solo = vec![selector("High")];
    run_render_command(&soloed).expect("render");

    assert_eq!(before, read(&input), "the input must be byte-identical");
}

/// The receipt records the invocation so it can be re-run. An argv array means
/// a path containing spaces survives without any quoting rules to get wrong.
#[test]
fn paths_with_spaces_round_trip_in_the_receipt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("a project with spaces.json");
    write_project(&input, |_| {});

    let output = dir.path().join("an output with spaces.wav");
    let mut spaced = command(&input, output.clone());
    spaced.argv = vec![
        "pertylizer".to_string(),
        "render".to_string(),
        "--input".to_string(),
        input.to_string_lossy().into_owned(),
        "--output".to_string(),
        output.to_string_lossy().into_owned(),
    ];
    let receipt = run_render_command(&spaced).expect("render");

    assert_eq!(receipt.command, spaced.argv);
    assert!(receipt.command.iter().any(|a| a.contains(' ')));
    assert!(output.exists());
}

/// Arguments that cannot describe any render are rejected on their own terms,
/// before the project is even opened.
#[test]
fn impossible_arguments_are_rejected_up_front() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("never-read.json");

    let mut zero = command(&missing, dir.path().join("zero.wav"));
    zero.seconds = Seconds::ZERO;
    assert!(matches!(
        run_render_command(&zero),
        Err(RenderError::InvalidDuration { .. })
    ));

    let mut negative_tail = command(&missing, dir.path().join("tail.wav"));
    negative_tail.tail = Seconds::new(-1.0);
    assert!(matches!(
        run_render_command(&negative_tail),
        Err(RenderError::InvalidDuration { .. })
    ));

    let mut absurd_rate = command(&missing, dir.path().join("rate.wav"));
    absurd_rate.sample_rate = 1;
    assert!(matches!(
        run_render_command(&absurd_rate),
        Err(RenderError::InvalidSampleRate(1))
    ));

    // The renderer would clamp this and warn. The command refuses, so nobody
    // compares five minutes of audio against the ten they asked for.
    let mut too_long = command(&missing, dir.path().join("long.wav"));
    too_long.seconds = Seconds::new(600.0);
    assert!(matches!(
        run_render_command(&too_long),
        Err(RenderError::DurationTooLong {
            what: "--seconds",
            ..
        })
    ));

    // Nothing downstream clamps a tail: its frame count goes straight into a
    // buffer allocation, so an unbounded one aborts the process rather than
    // returning an error.
    let mut absurd_tail = command(&missing, dir.path().join("tail.wav"));
    absurd_tail.tail = Seconds::new(100_000.0);
    assert!(matches!(
        run_render_command(&absurd_tail),
        Err(RenderError::DurationTooLong {
            what: "--tail-seconds",
            ..
        })
    ));

    // The rate ceiling is checked before the size guard, and 384 kHz is now
    // above it (`LIMIT-0004`), so this request is refused for the rate rather
    // than for the product it would have allocated.
    let mut huge = command(&missing, dir.path().join("huge.wav"));
    huge.seconds = Seconds::new(300.0);
    huge.sample_rate = 384_000;
    assert!(matches!(
        run_render_command(&huge),
        Err(RenderError::InvalidSampleRate(384_000))
    ));
}

/// Rendering over the input would destroy the project *and* still produce a
/// clean-looking receipt, because the input digest is taken before the write.
#[test]
fn an_output_that_is_the_input_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("fixture.json");
    write_project(&input, |_| {});
    let before = read(&input);

    let mut over_input = command(&input, input.clone());
    over_input.result_json = None;
    assert!(matches!(
        run_render_command(&over_input),
        Err(RenderError::OutputOverwritesInput { .. })
    ));

    let mut receipt_over_input = command(&input, dir.path().join("out.wav"));
    receipt_over_input.result_json = Some(input.clone());
    assert!(matches!(
        run_render_command(&receipt_over_input),
        Err(RenderError::OutputOverwritesInput { .. })
    ));

    // And the WAV and the receipt may not be the same file either, or whichever
    // lands second replaces the other.
    let collision = dir.path().join("both.out");
    let mut same_path = command(&input, collision.clone());
    same_path.result_json = Some(collision);
    assert!(matches!(
        run_render_command(&same_path),
        Err(RenderError::OutputCollision { .. })
    ));

    // Neither output exists yet, so the collision guard cannot lean on
    // `canonicalize` — two spellings of one path must still be caught, or the
    // receipt would land on top of the WAV it describes.
    let spelled = dir.path().join("spelled.out");
    let mut same_file_two_ways = command(&input, spelled.clone());
    same_file_two_ways.result_json = Some(dir.path().join(".").join("spelled.out"));
    assert!(matches!(
        run_render_command(&same_file_two_ways),
        Err(RenderError::OutputCollision { .. })
    ));

    assert_eq!(before, read(&input), "the project must be untouched");
}

/// A non-default sample rate has to reach the WAV header and the receipt, not
/// just the request — the consumer decodes against what the receipt claims.
#[test]
fn a_non_default_sample_rate_is_honoured() {
    let dir = tempfile::tempdir().expect("tempdir");
    let input = dir.path().join("fixture.json");
    write_project(&input, |_| {});

    let output = dir.path().join("48k.wav");
    let mut at_48k = command(&input, output.clone());
    at_48k.sample_rate = 48_000;
    let receipt = run_render_command(&at_48k).expect("render at 48 kHz");

    assert_eq!(receipt.audio.sample_rate, 48_000);
    let spec = hound::WavReader::open(&output)
        .expect("readable WAV")
        .spec();
    assert_eq!(spec.sample_rate, 48_000);
    let expected = (0.5 * 48_000.0) as u64;
    assert!(
        receipt.audio.frames.abs_diff(expected) <= 1,
        "expected ~{expected} frames at 48 kHz, got {}",
        receipt.audio.frames
    );
}
