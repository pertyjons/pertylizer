#!/usr/bin/env bash
# Assemble a Pertylizer release package directory and zip it.
#
# Usage: packaging/assemble.sh <os> <arch>
#   os:   linux | macos
#   arch: x86_64 | aarch64
#
# Expects release binaries already built:
#   target/release/pertylizer
#   visualizer/target/release/pertylizer-visualizer
set -euo pipefail

OS="$1"
ARCH="$2"

VERSION=$(grep '^version' crates/pertylizer/Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
PKG="pertylizer-${VERSION}-${OS}-${ARCH}"

rm -rf "$PKG" "${PKG}.zip"
mkdir -p "$PKG"

# --- Binaries -------------------------------------------------------------
if [ "$OS" = "macos" ]; then
  # Wrap the synth in a clickable .app bundle.
  APP="$PKG/Pertylizer.app"
  mkdir -p "$APP/Contents/MacOS"
  cp target/release/pertylizer "$APP/Contents/MacOS/pertylizer"
  chmod +x "$APP/Contents/MacOS/pertylizer"
  sed "s/__VERSION__/${VERSION}/g" packaging/macos/Info.plist >"$APP/Contents/Info.plist"
else
  cp target/release/pertylizer "$PKG/pertylizer"
  chmod +x "$PKG/pertylizer"
fi
cp visualizer/target/release/pertylizer-visualizer "$PKG/pertylizer-visualizer"
chmod +x "$PKG/pertylizer-visualizer"

# --- Examples -------------------------------------------------------------
cp -r assets/examples "$PKG/examples"

# --- Config + docs --------------------------------------------------------
cp packaging/README.md "$PKG/README.md"
cp packaging/pertylizer.toml "$PKG/pertylizer.toml"

# --- MCP configs (folder + project-local convenience copies) --------------
cp -r packaging/mcp "$PKG/mcp"
cp packaging/mcp/claude-code/.mcp.json "$PKG/.mcp.json"
mkdir -p "$PKG/.gemini"
cp packaging/mcp/gemini/settings.json "$PKG/.gemini/settings.json"

# --- .ai scaffold ---------------------------------------------------------
mkdir -p "$PKG/.ai"
cp packaging/ai-scaffold/README.md "$PKG/.ai/README.md"

# --- Zip (zip preserves the executable bit) -------------------------------
zip -r -q "${PKG}.zip" "$PKG"
echo "Built ${PKG}.zip"
ls -lh "${PKG}.zip"

# Export the package name for later workflow steps (if running in CI).
if [ -n "${GITHUB_ENV:-}" ]; then
  echo "PKG_NAME=$PKG" >>"$GITHUB_ENV"
fi
