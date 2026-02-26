#!/usr/bin/env python3
"""Simple CLI for calling MCP tools on the modular synth via Streamable HTTP.

Usage:
    mcp-call.py <tool_name> [key=value ...]

Values starting with [ or { are parsed as JSON (for arrays/objects).
Other values are auto-detected as int, float, or string.
Output is the raw JSON text from the tool response.
Exit code 0 on success, 1 on error.

Examples:
    # Synth engine
    mcp-call.py list_modules instrument_id=0
    mcp-call.py note_on channel=1 note=60 velocity=100
    mcp-call.py set_parameter instrument_id=0 module_id=osc-1 param_name=Level value=0.8

    # Sequencer (single-item)
    mcp-call.py get_song_info
    mcp-call.py set_song_tempo bpm=140
    mcp-call.py create_pattern name=Verse length_beats=4.0
    mcp-call.py add_note pattern_id=0 pitch=60 start_beat=0.0 duration_beats=1.0 velocity=100
    mcp-call.py create_track name=Lead instrument_id=0
    mcp-call.py place_pattern pattern_id=0 track_id=0 start_beat=0.0

    # Sequencer (batch) — use JSON for array values
    mcp-call.py add_notes pattern_id=0 'notes=[{"pitch":60,"start_beat":0.0,"duration_beats":1.0},{"pitch":64,"start_beat":1.0,"duration_beats":1.0}]'
    mcp-call.py clear_pattern pattern_id=0
    mcp-call.py create_patterns 'patterns=[{"name":"Verse","length_beats":4.0,"notes":[{"pitch":60,"start_beat":0.0,"duration_beats":1.0}]}]'
    mcp-call.py create_tracks 'tracks=[{"name":"Lead"},{"name":"Bass","instrument_id":0}]'
    mcp-call.py place_patterns 'placements=[{"pattern_id":0,"track_id":0,"start_beat":0.0}]'

    # Transport
    mcp-call.py seq_play
    mcp-call.py seq_stop
    mcp-call.py seq_seek beat=4.0
"""

import json
import sys
import urllib.request
import urllib.error

MCP_URL = "http://127.0.0.1:9850/mcp"


def parse_value(v):
    """Auto-detect JSON, int, float, or string."""
    if v.startswith("[") or v.startswith("{"):
        try:
            return json.loads(v)
        except json.JSONDecodeError:
            pass
    try:
        return int(v)
    except ValueError:
        pass
    try:
        return float(v)
    except ValueError:
        pass
    return v


def mcp_post(payload, session_id=None):
    """POST a JSON-RPC message to the MCP HTTP endpoint."""
    data = json.dumps(payload).encode()
    headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"}
    if session_id:
        headers["Mcp-Session-Id"] = session_id
    req = urllib.request.Request(MCP_URL, data=data, headers=headers, method="POST")
    resp = urllib.request.urlopen(req, timeout=10)
    sid = resp.headers.get("Mcp-Session-Id", session_id)
    body = resp.read().decode()
    return body, sid


def parse_sse_response(body):
    """Parse SSE stream and extract JSON-RPC response."""
    for line in body.splitlines():
        if line.startswith("data: "):
            try:
                return json.loads(line[6:])
            except json.JSONDecodeError:
                continue
    return None


def main():
    if len(sys.argv) < 2:
        print("Usage: mcp-call.py <tool_name> [key=value ...]", file=sys.stderr)
        sys.exit(1)

    tool = sys.argv[1]
    args = {}
    for arg in sys.argv[2:]:
        if "=" in arg:
            k, v = arg.split("=", 1)
            args[k] = parse_value(v)
        else:
            print(f"Invalid argument (expected key=value): {arg}", file=sys.stderr)
            sys.exit(1)

    try:
        # Initialize
        body, session_id = mcp_post({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "mcp-call", "version": "1.0"},
            },
        })

        # Initialized notification
        mcp_post(
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            session_id,
        )

        # Tool call
        body, _ = mcp_post({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": tool, "arguments": args},
        }, session_id)

    except urllib.error.URLError as e:
        print(f"Error: cannot connect to synth at {MCP_URL}: {e.reason}", file=sys.stderr)
        sys.exit(1)

    try:
        # Response may be direct JSON or SSE
        try:
            data = json.loads(body)
        except json.JSONDecodeError:
            data = parse_sse_response(body)
            if data is None:
                print(body, file=sys.stderr)
                sys.exit(1)

        text = data["result"]["content"][0]["text"]
        is_error = data["result"].get("isError", False) or text.startswith("Error:")
        print(text)
        sys.exit(1 if is_error else 0)
    except (KeyError, IndexError, TypeError):
        print(body, file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
