#!/usr/bin/env bash
# Run Pertylizer for development with the egui inspection plugin enabled, so the
# `egui` MCP server (egui-mcp) can drive the live UI over AccessKit.
#
# Compiles in the opt-in `egui-inspection` cargo feature and sets EGUI_INSPECTION=1
# so eframe's inspection plugin binds 127.0.0.1:5719. The feature is kept out of
# the default build (release/CI binaries don't link accesskit), so use THIS script
# whenever you want an AI agent to inspect/drive the running app.
#
# Extra args are forwarded to `cargo run`, e.g.:  scripts/run-inspect.sh --release
set -euo pipefail
cd "$(dirname "$0")/.."
exec env EGUI_INSPECTION=1 cargo run --features egui-inspection "$@"
