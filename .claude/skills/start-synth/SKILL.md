---
name: start-synth
description: Start the modular synth with MCP support so Claude can inspect and control it
disable-model-invocation: true
allowed-tools: Bash
---

Start the modular synth with MCP support enabled.

## Steps

1. Check if port 9850 is already in use:
   ```bash
   ss -tlnp | grep 9850
   ```

2. If a synth is already running on port 9850, ask the user if they want to kill it and restart.

3. Build and run with MCP feature:
   ```bash
   cargo run --features mcp
   ```
   Run this in the background so Claude can continue working.

4. Wait 3 seconds, then verify the MCP server is listening:
   ```bash
   ss -tlnp | grep 9850
   ```

5. Report status to the user:
   - Whether GUI started
   - Whether MCP server is listening on 127.0.0.1:9850
   - Remind that Claude Code can connect via the bridge binary configured in `.claude/settings.json`
