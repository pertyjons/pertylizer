---
name: start-synth
description: Start the modular synth with MCP support so Claude can inspect and control it
disable-model-invocation: true
allowed-tools: Bash
---

Start the modular synth with MCP support enabled.

## Steps

1. Check if a synth is already running on port 9850 and kill it:
   ```bash
   PID=$(ss -tlnp | grep 9850 | grep -oP 'pid=\K\d+')
   if [ -n "$PID" ]; then
     kill "$PID"
     sleep 1
   fi
   ```

2. Build and run with MCP feature:
   ```bash
   cargo run --features mcp
   ```
   Run this in the background so Claude can continue working.

3. Wait 3 seconds, then verify the MCP server is listening:
   ```bash
   ss -tlnp | grep 9850
   ```

4. List available MCP tools:
   ```bash
   .claude/skills/start-synth/mcp-call.py list_module_types | head -5
   ```
   If this works, the MCP connection is verified.

5. Report status to the user:
   - Whether GUI started
   - Whether MCP server is listening on 127.0.0.1:9850
   - Confirm MCP tools are available

## Calling MCP Tools

Use the helper script for all MCP calls. It handles TCP connection and handshake automatically:

```bash
MCP=".claude/skills/start-synth/mcp-call.py"

# Query
$MCP list_instruments
$MCP list_modules instrument_id=0
$MCP get_module_info instrument_id=0 module_id=osc-1
$MCP get_connections instrument_id=0
$MCP get_engine_status

# Build patches
$MCP clear_graph instrument_id=0
$MCP add_module instrument_id=0 module_type=osc
$MCP set_parameter instrument_id=0 module_id=osc-1 param_name=Waveform value=2
$MCP connect instrument_id=0 from_module=osc-1 from_port=out to_module=flt-1 to_port=in
$MCP load_example_patch name="Moog Resonant Sweep"

# Play notes
$MCP note_on channel=1 note=60 velocity=100
$MCP note_off channel=1 note=60
```

For batch operations (building a full patch), use a Python script that imports `socket` directly to avoid repeated handshakes. See `mcp-call.py` for the protocol details.
