# Sound Core Target

Sound Core compiles semantic graph input into an immutable prepared plan and
separate mutable runtime state. The audio thread executes compact numeric
identities over preallocated storage; it does not resolve names, traverse the
authoring graph, allocate, lock, perform I/O, or mutate canonical project state.

The same renderer serves live playback, offline rendering, analysis, CLI use,
and future runtime-library consumers. Host adapters own devices, external
clocks, protocols, files, worker scheduling, and authorization.

Compilation owns validation, scheduling, resource admission, buffer assignment,
prepared data, and structured diagnostics. Rendering owns bounded event
application and DSP. Telemetry is lossy runtime observation and never persisted
truth.

The current executable contract is
[`../specs/spec-sound-core-render-contract.md`](../specs/spec-sound-core-render-contract.md).
