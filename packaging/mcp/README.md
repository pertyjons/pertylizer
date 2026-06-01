# Driving Pertylizer from an AI CLI (MCP)

Pertylizer runs an MCP server while the GUI is open, at:

```
http://127.0.0.1:9850/mcp
```

The port comes from `[mcp] port` in `pertylizer.toml` (next to the executable).
**If you change the port there, update the URL in the config you use below.**

The fastest path: launch your AI CLI **from the unpacked package directory**.
It already contains a project-local `.mcp.json` (Claude Code) and
`.gemini/settings.json` (Gemini), so the `synth` server is picked up
automatically — start Pertylizer, then run `claude` or `gemini` in that folder.

The `mcp/` folder holds ready-made configs for each client:

| Client            | File                          | Where it goes                                  |
|-------------------|-------------------------------|------------------------------------------------|
| Claude Code       | `claude-code/.mcp.json`       | Project root (already at package root)         |
| Gemini CLI        | `gemini/settings.json`        | `.gemini/settings.json` (already at root) or `~/.gemini/settings.json` |
| Antigravity       | `antigravity/mcp.json`        | Antigravity's MCP settings (`mcpServers`)      |
| Codex CLI         | `codex/config.toml`           | Merge into `~/.codex/config.toml`              |

For Claude Code / Gemini you can also merge the `synth` entry into your global
config instead of running from this directory.
