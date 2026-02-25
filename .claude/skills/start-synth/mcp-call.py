#!/usr/bin/env python3
"""Simple CLI for calling MCP tools on the modular synth.

Usage:
    mcp-call.py <tool_name> [key=value ...]

Examples:
    # Synth engine
    mcp-call.py list_modules instrument_id=0
    mcp-call.py note_on channel=1 note=60 velocity=100
    mcp-call.py set_parameter instrument_id=0 module_id=osc-1 param_name=Level value=0.8

    # Sequencer
    mcp-call.py get_song_info
    mcp-call.py set_song_tempo bpm=140
    mcp-call.py create_pattern name=Verse length_beats=4.0
    mcp-call.py add_note pattern_id=0 pitch=60 start_beat=0.0 duration_beats=1.0 velocity=100
    mcp-call.py create_track name=Lead instrument_id=0
    mcp-call.py place_pattern pattern_id=0 track_id=0 start_beat=0.0
    mcp-call.py seq_play
    mcp-call.py seq_stop
    mcp-call.py seq_seek beat=4.0

Values are auto-detected as int, float, or string.
Output is the raw JSON text from the tool response.
Exit code 0 on success, 1 on error.
"""

import socket
import json
import sys
import time


def parse_value(v):
    """Auto-detect int, float, or string."""
    try:
        return int(v)
    except ValueError:
        pass
    try:
        return float(v)
    except ValueError:
        pass
    return v


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

    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        s.connect(("127.0.0.1", 9850))
    except ConnectionRefusedError:
        print("Error: synth not running on port 9850", file=sys.stderr)
        sys.exit(1)
    s.settimeout(5)

    def send(msg):
        s.sendall((json.dumps(msg) + "\n").encode())

    def recv(timeout=2):
        s.settimeout(timeout)
        buf = b""
        try:
            while True:
                chunk = s.recv(8192)
                if not chunk:
                    break
                buf += chunk
        except socket.timeout:
            pass
        return buf.decode()

    # Handshake
    send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "mcp-call", "version": "1.0"},
    }})
    time.sleep(0.3)
    recv(0.5)
    send({"jsonrpc": "2.0", "method": "notifications/initialized"})
    time.sleep(0.1)

    # Tool call
    send({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
        "name": tool,
        "arguments": args,
    }})
    time.sleep(0.3)
    raw = recv(2)
    s.close()

    try:
        data = json.loads(raw)
        text = data["result"]["content"][0]["text"]
        is_error = data["result"].get("isError", False) or text.startswith("Error:")
        print(text)
        sys.exit(1 if is_error else 0)
    except (json.JSONDecodeError, KeyError, IndexError):
        print(raw, file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
