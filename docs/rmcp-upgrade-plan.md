# rmcp Upgrade Plan: 0.17 → 1.2.0

## Background

rmcp 1.2.0 released 2026-03-11. Major version bump (0.x → 1.x) — API is now considered stable.

## Step 1: Core Upgrade (Required)

1. Bump `rmcp` to `"1.2"` in workspace `Cargo.toml`
2. Fix compilation errors (import paths may have changed)
3. Verify feature flags still exist: `server`, `transport-io`, `transport-streamable-http-server`
4. Run `cargo build`, `cargo clippy`, `cargo test`

## Step 2: Enhanced Server Metadata

Fill in the new `Implementation` fields in `get_info()`:

- `title`: `"Pertylizer"`
- `description`: `"Modular synthesizer with 35 voice modules, 21 effects, and MCP integration"`
- `website_url`: `"https://github.com/pertyjons/pertylizer"`
- `icons`: optional, skip for now

## Step 3: Cancellation Token for HTTP Server

Replace `server.serve(transport)` with `server.serve_with_ct(transport, ct)` in `serve_http()` for clean shutdown support.

## Future Opportunities (Not in Initial Upgrade)

### RawAudioContent — Return Audio in Tool Responses

New content type for audio (base64 + mime_type). Could let tools like `note_on` return audio preview clips. Very relevant for a synthesizer.

### Task Management — Async Long-Running Operations

`ServerCapabilities` now supports `tasks`. Enables:
- `enqueue_task()` for long-running operations
- `list_tasks()`, `get_task_info()`, `get_task_result()`, `cancel_task()`

Relevant for: `build_instruments`, `load_project`, `optimize_project`.

### Completions — Parameter Autocompletion

New `completions` capability in `ServerCapabilities`. Could provide autocompletion for parameter names, module types, etc. in MCP clients.

### Elicitation — Interactive User Input

Server can request structured input from user during tool execution. Not needed currently — our tools take all parameters directly.

### OAuth 2.0 Authentication

Not relevant for local synth.

## Breaking Change Risk Assessment

| Area | Risk | Detail |
|------|------|--------|
| `ServerInfo` | Low | Now a type alias for `InitializeResult` — works with `..Default::default()` |
| `ServerHandler` trait | Low | New methods all have default implementations |
| `Implementation` struct | Low | New fields are all `Option` |
| Feature flags | Check | Names should be unchanged |
| Import paths | Check | `rmcp::handler::server::router::tool::ToolRouter` etc. may have moved |
