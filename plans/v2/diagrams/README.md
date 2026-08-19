# Core V2 Architecture Diagrams

This directory contains visual summaries of the target architecture described
under [`../architecture/`](../architecture/README.md).

The diagrams are explanatory, not normative. If a diagram conflicts with the
target description or a current specification, the specification wins and the
diagram should be updated.

## Diagrams

- [Core responsibilities](01-core-responsibilities.svg) — authority and
  responsibility across Project, Application, and Sound Core V2.
- [Sound Core real-time boundary](02-sound-core-realtime.svg) — the audio-thread
  boundary, communication lanes, immutable plan swapping, and off-thread work.
- [Frontend, jobs, and telemetry](03-frontends-jobs-telemetry.svg) — how GUI,
  MCP, and CLI share operations, jobs, session state, and telemetry.

Open the [HTML overview](index.html) to view all three diagrams on one page.

The SVG files are the editable source and are versioned. `index.html` is a
convenience preview and may be regenerated or omitted. There are no exported
raster copies: an SVG renders everywhere the repository is read, and a PNG beside
it would be one more thing to keep in step with the source.
