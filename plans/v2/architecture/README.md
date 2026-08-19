# Core V2 Architecture

These documents describe the target shape and dependency direction. They are
explanatory, not normative: current implementation contracts live under
[`../specs/`](../specs/README.md), and durable rationale lives in
[`../decisions/`](../decisions/README.md).

Start with:

- [Project Core](project-core.md) — canonical authored state and assets;
- [Application Core](application-core.md) — operations, revisions, history, and
  compilation coordination;
- [Sound Core](sound-core.md) — compiled plans and bounded rendering.

The migration order and exit outcomes are in [`../ROADMAP.md`](../ROADMAP.md).

```text
GUI / MCP / CLI / importers
             |
             v
      Application Core
      /       |       \
     v        v        v
Project Core  jobs   compile coordinator
     |                    |
     v                    v
Project I/O          Sound Core plan
                          |
                          v
                  host audio / offline output
```

Host configuration, runtime session state, jobs, and telemetry are adjacent to
the canonical project document; none becomes persisted truth merely because a
frontend can observe it.
