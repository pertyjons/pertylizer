# Core V2 Architecture Diagrams

This directory contains visual summaries of the architecture defined in
[`../master-plan.md`](../master-plan.md).

The diagrams are explanatory, not normative. If a diagram conflicts with the
master plan, an accepted ADR, or a current specification, the authoritative
document wins and the diagram should be updated.

## Diagrams

- [Core responsibilities](01-core-responsibilities.svg) — authority and
  responsibility across Project, Application, and Sound Core V2.
- [Sound Core real-time boundary](02-sound-core-realtime.svg) — the audio-thread
  boundary, communication lanes, immutable plan swapping, and off-thread work.
- [Frontend, jobs, and telemetry](03-frontends-jobs-telemetry.svg) — how GUI,
  MCP, and CLI share operations, jobs, session state, and telemetry.

Open the [HTML overview](index.html) to view all three diagrams on one page.

The SVG files are the editable source and should be versioned. `index.html` and
the PNG files are convenience previews and may be regenerated or omitted from
the repository.
