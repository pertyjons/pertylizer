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

# Sequencer: Song
$MCP get_song_info
$MCP set_song_name name="My Song"
$MCP set_song_tempo bpm=140

# Sequencer: Patterns
$MCP list_patterns
$MCP create_pattern name="Verse" length_beats=4.0
$MCP delete_pattern pattern_id=0

# Sequencer: Notes (beats: 1.0=quarter, 0.5=eighth, 0.25=sixteenth)
$MCP list_notes pattern_id=0
$MCP add_note pattern_id=0 pitch=60 start_beat=0.0 duration_beats=1.0 velocity=100
$MCP update_note pattern_id=0 note_id=0 pitch=62 velocity=110
$MCP remove_note pattern_id=0 note_id=0

# Sequencer: Tracks
$MCP list_tracks
$MCP create_track name="Lead" instrument_id=0

# Sequencer: Arrangement
$MCP list_arrangement
$MCP place_pattern pattern_id=0 track_id=0 start_beat=0.0
$MCP remove_placement pattern_id=0 track_id=0 start_beat=0.0

# Sequencer: Transport
$MCP seq_play
$MCP seq_stop
$MCP seq_seek beat=4.0
```

## Batch Operations (Sequencer)

Use JSON arrays for batch parameters. Quote the JSON value with single quotes in bash.

```bash
MCP=".claude/skills/start-synth/mcp-call.py"

# Add multiple notes to a pattern in one call
$MCP add_notes pattern_id=0 'notes=[{"pitch":60,"start_beat":0.0,"duration_beats":1.0},{"pitch":64,"start_beat":1.0,"duration_beats":0.5,"velocity":110}]'

# Update multiple notes (only provided fields are changed)
$MCP update_notes pattern_id=0 'updates=[{"note_id":0,"pitch":62},{"note_id":1,"velocity":80}]'

# Replace all notes in a pattern (clear + add)
$MCP replace_notes pattern_id=0 'notes=[{"pitch":60,"start_beat":0.0,"duration_beats":1.0}]'

# Clear all notes from a pattern
$MCP clear_pattern pattern_id=0

# Create multiple patterns (with optional inline notes)
$MCP create_patterns 'patterns=[{"name":"Verse","length_beats":4.0,"notes":[{"pitch":60,"start_beat":0.0,"duration_beats":1.0}]},{"name":"Chorus","length_beats":8.0}]'

# Create multiple tracks
$MCP create_tracks 'tracks=[{"name":"Lead","instrument_id":0},{"name":"Bass"}]'

# Place multiple patterns in the arrangement
$MCP place_patterns 'placements=[{"pattern_id":0,"track_id":0,"start_beat":0.0},{"pattern_id":1,"track_id":1,"start_beat":4.0}]'

# Build a full song in one call (replaces current song)
# Placements use array indices: pattern_index/track_index refer to position in the arrays
$MCP set_song 'name=My Song' tempo=140 \
  'patterns=[{"name":"Verse","length_beats":4.0,"notes":[{"pitch":60,"start_beat":0.0,"duration_beats":1.0},{"pitch":64,"start_beat":1.0,"duration_beats":1.0}]},{"name":"Chorus","length_beats":8.0,"notes":[{"pitch":67,"start_beat":0.0,"duration_beats":2.0}]}]' \
  'tracks=[{"name":"Lead","instrument_id":0},{"name":"Bass"}]' \
  'placements=[{"pattern_index":0,"track_index":0,"start_beat":0.0},{"pattern_index":1,"track_index":0,"start_beat":4.0}]'
```

### Batch response format

Batch operations return per-item results (partial success, no rollback):

```json
{
  "total": 3,
  "succeeded": 2,
  "failed": 1,
  "items": [
    {"index": 0, "success": true, "id": 0, "error": null},
    {"index": 1, "success": true, "id": 1, "error": null},
    {"index": 2, "success": false, "id": null, "error": "note not found: 99"}
  ]
}
```

`set_song` returns a summary with assigned IDs:

```json
{
  "patterns_created": 2,
  "tracks_created": 2,
  "notes_added": 5,
  "placements_created": 3,
  "pattern_ids": [0, 1],
  "track_ids": [0, 1],
  "errors": []
}
```
