# Project Core Target

Project Core is the canonical, deterministic document for authored musical and
editor state. It owns stable domain identities, graph and arrangement content,
automation, routing, asset references, and intentionally persisted editor
metadata.

It does not own device configuration, playback position, live voices, meters,
job progress, frontend interaction state, or prepared DSP caches.

Project I/O is a separate boundary that decodes a versioned envelope, converts
formats, validates references and invariants, and opens a document. Source
assets retain identity and provenance; prepared representations are derived and
cacheable. Saving snapshots canonical data rather than reconstructing it from a
frontend or engine.

Current normative contracts will live under [`../specs/`](../specs/README.md).
The state, identity, and capability inventories remain the evidence for what V1
must migrate or intentionally drop.
