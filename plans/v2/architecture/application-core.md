# Application Core Target

Application Core is the sole mutation boundary for Project Core. GUI, MCP, CLI,
importers, undo, recovery, and tests request typed operations against a project
revision rather than editing mirrors or sending persistence-shaped engine
commands.

An operation reports its resulting revision, effect, diagnostics, compile
impact, and affected stable identities. Transactions, optimistic concurrency,
history, dirty state, save coordination, jobs, and compilation all use the same
revision stream.

Runtime session commands and telemetry remain separate from project operations.
A failed compile leaves the editing revision visible while the last valid plan
continues to render. Long-running work is revision-pinned, cancellable, and
returns an explicit receipt.

Current normative contracts will live under [`../specs/`](../specs/README.md).
