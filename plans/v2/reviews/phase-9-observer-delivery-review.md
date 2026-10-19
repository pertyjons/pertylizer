# Phase 9 application observer delivery review

## Scope and stopping rule

This slice implements ADR-0089's compiled-application observation lane and CPAL
cold processor custody. It also implements the explicitly approved ADR-0088
feature forwarding boundary and delivers the separate physical-backend
instrumentation proposal. A false behavioral claim, ownership hole or uncontrolled
measurement blocks acceptance. The four remaining implementation outcomes in the
[six-point register](phase-9-six-point-software-completion.md) remain open.

## Independent reader and repairs

Codex authored the change. Fresh Claude Code invocations read the complete
uncommitted tracked and untracked scope with only Read/Grep/Glob, empty MCP
configuration and slash commands disabled; no reader delegated or edited.
Initial design review rejected a terminal mutex handoff and a reference-count
snapshot as proof of exclusive custody. The implementation instead uses a safe
owned atomic exchange, `Arc::try_unwrap`, separately owned fences and a single
bounded cold worker. The reader audited atomicbox 0.4.0's resolved source and
license and verified that the guard cannot become the mailbox's final owner.

The uncommitted review found missing CI execution of backend controls, completion
published before reservation release, an inadequate marker control, a malformed
ADR index and unrelated lockfile churn. Those are repaired. Focused reread found
no blocking defect. Its remaining test-strength finding is addressed by holding
a preceding processor destructor blocked while asserting Pending, and by an
unwinding-destructor and unrelated-unwind controls. Only atomicbox's package and application edge remain
in the lockfile diff. Spawn failure is explicitly documented as process-lifetime
refusal. No feature claims a mixed application route already exists.

## Verification

The complete repository gate passes, including evidence, workspace build/tests,
Clippy, documentation, feature configurations and Rust 1.98. The gate now has
24 commands: the previous 19 plus one backend-retirement command and four
mixed-application commands mirrored in CI and CLAUDE.md. Affected checks are
repeated after the final test-only repairs.

The five backend controls cover both native-parent/closure destruction orders,
delayed callbacks at the eight-registration bound, stalled destruction, failed
registration and an unwinding destructor followed by successful reuse of the same
worker. Application controls use actual audio threads for subscription lifecycle,
backpressure, stopped/running handoff and lineage ending. They also check faulted
ending, foreign joins, request exhaustion, cancelled custody, bounded reads and
passive sound against an unobserved renderer. The four-rate lifecycle control
compares samples exactly and guards callbacks against allocation/deallocation.

## Retained boundaries

The switch's marker observes field destruction, not enclosing-box destruction,
thread identity or native-device quiescence. CPAL/CoreAudio native teardown
assumptions remain physical-platform work. Standalone examples retain their
separate guards. The instrumentation proposal adds no unsafe code and requires
separate approval before implementation. Canonical project revision stays Phase
0B/10A; production calibration and physical clock qualification remain open.
