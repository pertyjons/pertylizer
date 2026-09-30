# EVD-0025: Song-Playback Producer Partition

| Field | Value |
|---|---|
| ID | EVD-0025 |
| Status | Complete |
| Phase | 09 |
| Created | 2026-09-29 |
| Last reviewed | 2026-09-30 |
| Supersedes | — |
| Superseded by | — |
| Source revision | `41fdc24d65ad14dfd87ac77e7f17730befaef081` |
| Retention | Until phase exit |
| Related | ADR-0054, ADR-0046, ADR-0076, ADR-0077, EVD-0021, EVD-0024 |
| Artifacts | [Full run](EVD-0025-run.txt), [extracted rows](EVD-0025-results.txt) and [checksums](EVD-0025-SHA256SUMS.txt) |

## Question and falsifier

ADR-0054 clause 3 forbids a production live adapter from starting under the provisional
partition. ADR-0077 decision 6 applies that to the application's V2 song playback. The
question is narrower than EVD-0024's: **can the producer partition that compiled song
playback admits be reselected from observed occupancy and admitted bounds**, so that the
song path, and only it, may start?

The partition is the one the song path can reach: a lowered plan's compiled producer and
the session transport (play, stop). Compiled note-offs charge the Compiled class and a
compiled producer holds no release entitlement, so the path cannot reach the Release share,
release holds, live ingress, the authored runtime or a renderer-internal producer. This
record must show that rather than assume it. It qualifies nothing for a partition that
enables any of those; EVD-0024's conclusion stands for them.

**Why a measured set can qualify unseen projects.** Any project the application opens can
reach the song path if it lowers, not only the projects measured here. Qualification
therefore rests on bounds that are enforced for every plan before playback, and the
measurement shows that real projects sit within them and that nothing else is charged:
admission refuses a compiled event window over the compiled share
(`AdmittedCompiledStream::admit`), and session admission refuses a plan whose activation
cost exceeds the session share. Both refusals happen before a callback exists.

The selection rule precedes collection:

- **Compiled share:** the current 96 is selected, as an enforced bound, if every measured
  observed compiled high water is at or below it.
- **Session share:** the current 128 is selected, as an enforced bound, if every measured
  observed session high water is at or below it (EVD-0021 chose 128 from catch-up requests).
- **Per-quantum cap:** the current 360 is selected if every observed external total is at or
  below it. It cannot be exceeded while both enforced shares hold, since they sum to 224.
- **Every other share, the release-hold capacity and the ingress depth** stay at their
  provisional values and are recorded as *not reachable by this partition*, never as
  measured zero-cost or qualified.

The preferred conclusion is **wrong** if any of these occur:

- **F1 — unenforced compiled bound.** A compiled stream with one event more than the share
  in one render quantum is not refused with `WindowOverShare` before playback.
- **F2 — unenforced session bound.** A measured plan compiled under a profile whose session
  share is one below its activation cost is not refused by compilation on
  `SessionEventShare`. A refusal elsewhere, such as at play validation, does not count.
- **F3 — wrong partition.** Any class other than Compiled or Session shows a nonzero high
  water in any run; any lowered plan declares a non-compiled note producer; or any lowered
  plan's admission report requests a nonzero `InternalEventShare`. Renderer-internal events
  use their own arena and never reach the external arbiter, so the report row, not the
  high water, is what shows that none is declared.
- **F4 — broken instrument.** A run's observed compiled high water differs from the static
  peak over quantum-aligned windows of the same plan's event positions, or a stopped
  control run reports any nonzero high water. *Clarified 2026-09-30, after a trial run and
  before the retained collection:* this item first said "sliding", but the arbiter charges
  per rendered quantum and every run activates on a quantum boundary, so the aligned peak is
  the instrument's reference. The every-phase sliding peak is admission's bound, and F5 now
  also requires it to stay within the compiled share.
- **F5 — capacity exceeded.** Any observed value, or any plan's every-phase sliding compiled
  peak, exceeds the value the rule selects.
- **F6 — unfaithful or incomplete run.** Any callback fails or faults the session; any of
  play, stop, replay or the end stop is not applied; the renderer reports a publication
  fault; audio before the stop differs from the lowering's offline render one quantum after
  the play boundary; audio from the replay on differs between callbacks of the maximum block
  and an irregular partition of the same frames, or is non-finite, or is silent; or any
  callback allocates or frees; or the replay's applied position differs from the stop's.
  A replay is a new activation with ADR-0050's catch-up, not a continuation of the voices'
  state, so it has no uninterrupted offline reference. F6 establishes that each run executed
  the declared path and resumed where it stopped; the correctness of the values catch-up
  restores is ADR-0050's contract, tested with it, and does not affect the occupancy this
  record measures, which is charged whatever those values are.

The result is `Supported` only if F1–F6 do not fire over every measured project and every
profile below. It is `Not supported` if a completed run fires one. It is `Inconclusive`
if a required project or profile is missing, or no measured project lowers.

## Inputs and controls

- **Projects:** every `.ptz` and `.ptz.zip` file anywhere in the repository, each lowered
  as a whole through `lower_project`; those that produce a plan are measured, and the rest
  are listed with their first refusal. The set is recorded with the results.
- **Profiles:** 44.1, 48 and 96 kHz; stereo; maximum blocks of 64, 256 and 8192 frames,
  through `HostProfile::harness`, so every run uses the provisional partition unchanged.
- **Transport:** play from the start, stop near the middle, play again from the stopped
  position, then the automatic stop at the arrangement's end. Play, stop and replay are
  scheduled at fixed quantum boundaries so that both callback partitions receive identical
  commands. Every run renders once with callbacks of the profile's maximum block and once
  with a fixed irregular partition of the same frames. Every command's receipt must be
  applied.
- **Controls:** a stopped run that renders without playing (all high waters zero); the F1
  admission control over a synthetic stream at the share and one above it; the F2
  compilation control over a measured plan under a profile whose session share is one below
  its activation cost; the F4 static peak per plan.

## Method

`SessionAudio` gains read-only `high_water(class)` and `high_water_external_total()`
accessors over its arbiter, never reset during a run, and a read-only view of the renderer's
publication-fault counter; `SongAudio` passes them through. Each run reads them after the
last callback and checks every callback's success. F4's static reference counts lowered
event positions per quantum-aligned window; separately, the every-phase sliding peak is
checked against the compiled share. A project is measured under every profile in which it
lowers, and a profile in which it does not lower is reported as a refusal row rather than
omitted. Runs are reported per project and profile;
maxima from different runs are never added and called simultaneous. Timing, deadlines and
physical devices are outside this measurement.

## Reproduction

```text
CARGO_TARGET_DIR=<fresh> cargo test -p pertylizer --release --lib evd_0025_song_partition_matrix -- --ignored --nocapture
cargo test -p pertylizer --lib evd_0025
```

From `plans/v2/evidence/phase-09/`, regenerate and verify the retained rows:

```bash
rg '^scope=|^evd_0025' EVD-0025-run.txt > EVD-0025-results.txt
sha256sum -c EVD-0025-SHA256SUMS.txt
```

## Results

On 2026-09-29 (UTC) the matrix ran from committed revision `41fdc24d` in a fresh Cargo target
directory, recorded in the [full run](EVD-0025-run.txt) with command, times, Rust, Cargo and
Linux versions, and exit status zero. Its one changed worktree path is the run file itself,
created before the command; every source matched the revision. The
[extracted rows](EVD-0025-results.txt) are regenerated from it and both are covered by the
[checksum file](EVD-0025-SHA256SUMS.txt).

The scan found 28 saved projects. Four lower as a whole under every one of the nine profiles
(`instrument-inserts`, `mod-matrix`, `subtractive-voice`, `tempo-map-arrangement`); no
profile refused a project that lowers under another. The other 24 refuse, each with its
first named refusal in the rows. Every one of the 36 measured runs passed F3–F6 under both
callback partitions, and the fast controls F1, F2 and the stopped control pass in the
ordinary test suite.

| Project | Compiled high water | Session high water | External total | Session request |
|---|---|---|---|---|
| instrument-inserts | 2 | 29 | 31 | 29 |
| mod-matrix | 1 | 23 | 24 | 23 |
| subtractive-voice | 1 | 21 | 22 | 21 |
| tempo-map-arrangement | 1 | 21 | 22 | 21 |

Each value is identical across the nine profiles and both partitions. The session high water
equals each plan's activation cost. Authored-runtime, Live, Release and Internal high waters
are zero in every run, no plan declares a non-compiled producer or a hold, and every plan
requests zero internal events.

## Limitations

Only the song-playback partition is covered. The measured workload is the set of saved
projects that lower today, which is small; any other project that lowers is covered by the
two enforced bounds, not by this measurement. CPU cost, underruns and physical
timing are not measured.

## Conclusion

**Supported for the song-playback partition only.** Under the predeclared rule the compiled
share is selected at 96 and the session share at 128, each as a bound enforced before
playback (F1, F2), with the measured projects at 2 and 29; the per-quantum cap is selected at
360. Authored-runtime 48, live 32, internal 16, release 40, release-hold capacity 40 and the
ingress depth 32 are not reachable by this partition and remain provisional and unqualified;
any partition that enables them still needs its own selection under ADR-0054, and
EVD-0024's conclusion stands for them. ADR-0077's capacity gate may open for the song path
alone.
