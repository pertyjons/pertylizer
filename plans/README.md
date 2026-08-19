# Pertylizer Plans

This directory holds design and migration plans. `docs/history.md` records what shipped; these documents record what is
intended and why. Each plan states its own status and planning baseline — read that header before trusting the body,
because a plan is only as current as its last review against the code.

## The active architecture effort

**[v2/](v2/README.md) — Pertylizer Core V2.** The migration to a canonical project document, one application-operation
boundary, and a compiled audio engine. It is a coordination space rather than a single document: start at
[v2/README.md](v2/README.md), then [v2/NOW.md](v2/NOW.md) for what is active now. Anything touching project
ownership, save/load, the engine, or the mutation path should be checked against
its current specifications before it is designed twice.

## Backlog

| Document           | What it is                                                                     |
|--------------------|--------------------------------------------------------------------------------|
| [TODO.md](TODO.md) | The running backlog, organized by subsystem. Section numbers are never reused. |

## Feature and subsystem plans

| Document                                                 | Status                                                  |
|----------------------------------------------------------|---------------------------------------------------------|
| [mcp-agent-api-redesign.md](mcp-agent-api-redesign.md)   | Proposed; Phase 0 merged. Aligns with Core V2           |
| [game-runtime-library.md](game-runtime-library.md)       | Proposed. Assumes V1 `SynthEngine`; see Core V2 Part VI |
| [sample-view-expansion.md](sample-view-expansion.md)     | Draft. Sample workstation view                          |
| [egui-theme-architecture.md](egui-theme-architecture.md) | Planned. Unified GUI theming                            |
| [headless-render-cli.md](headless-render-cli.md)         | Implemented and merged; kept as the design record       |
| [perty-developer-analysis-tool.md](perty-developer-analysis-tool.md) | Proposed. Change-impact and real-time safety analysis |

Status here is a pointer, not the authority — the document's own header is. When a plan is fully delivered and has no
remaining rationale worth keeping, delete it rather than leaving a stale document; Git keeps the history.

## Relationship to Core V2

Core V2 does not replace these plans, but it does constrain several of them: it owns the project model, the mutation
boundary, the asset and tuning model, and the render/job contract.
[v2/ROADMAP.md](v2/ROADMAP.md) records the migration order, while current
contracts live under [v2/specs/](v2/specs/README.md). If a plan here contradicts
a current Core V2 specification, the specification wins and the plan is
updated.

The legacy master plan's
[coordination map](v2/master-plan.md#part-vi-coordination-with-existing-plans)
remains the navigation index for overlaps with the MCP redesign, game runtime,
headless tools, sampling/tuning work, OSC, visualization, and project safety.
It is historical guidance rather than a current contract; move a rule into a
current specification or roadmap outcome before implementation depends on it.
