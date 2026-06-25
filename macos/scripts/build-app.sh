#!/usr/bin/env bash
# Build AirJedi.app macOS application bundle
# Assembles the .app directory structure with binary, assets, and metadata

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MACOS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ROOT_DIR="$(cd "$MACOS_DIR/.." && pwd)"

APP_NAME="AirJedi"
BUILD_DIR="$MACOS_DIR/build"
APP_DIR="$BUILD_DIR/$APP_NAME.app"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_BIN_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"

BINARY_NAME="airjedi_bevy"
BUILD_MODE="${BUILD_MODE:-release}"
BINARY_PATH="$ROOT_DIR/target/$BUILD_MODE/$BINARY_NAME"
ICNS_FILE="$MACOS_DIR/icons/AppIcon.icns"
PLIST_TEMPLATE="$MACOS_DIR/Info.plist.template"

# Extract version from Cargo.toml
VERSION=$(grep '^version' "$ROOT_DIR/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')

echo "Building $APP_NAME v$VERSION..."

# Step 1: Build binary
echo "Step 1: Building $BUILD_MODE binary..."
if [ "$BUILD_MODE" = "release" ]; then
    (cd "$ROOT_DIR" && cargo build --release --no-default-features --features fusion)
else
    (cd "$ROOT_DIR" && cargo build)
fi

if [ ! -f "$BINARY_PATH" ]; then
    echo "Error: Binary not found at $BINARY_PATH"
    exit 1
fi

# Step 2: Create .app directory structure
echo "Step 2: Creating .app bundle structure..."
rm -rf "$APP_DIR"
mkdir -p "$MACOS_BIN_DIR"
mkdir -p "$RESOURCES_DIR"

# Step 3: Copy release binary
echo "Step 3: Copying binary..."
cp "$BINARY_PATH" "$MACOS_BIN_DIR/$BINARY_NAME"
chmod +x "$MACOS_BIN_DIR/$BINARY_NAME"

# Step 4: Copy assets next to binary (Bevy resolves assets/ relative to executable)
echo "Step 4: Copying assets..."
if [ -d "$ROOT_DIR/assets" ]; then
    rsync -a \
        --exclude='tiles' \
        --exclude='*.tile.*' \
        "$ROOT_DIR/assets/" "$MACOS_BIN_DIR/assets/"
else
    echo "Warning: assets directory not found at $ROOT_DIR/assets"
fi

# Step 5: Generate Info.plist from template
echo "Step 5: Generating Info.plist..."
if [ ! -f "$PLIST_TEMPLATE" ]; then
    echo "Error: Info.plist template not found at $PLIST_TEMPLATE"
    exit 1
fi
sed "s/__VERSION__/$VERSION/g" "$PLIST_TEMPLATE" > "$CONTENTS_DIR/Info.plist"

# Step 6: Copy icon
echo "Step 6: Copying icon..."
if [ -f "$ICNS_FILE" ]; then
    cp "$ICNS_FILE" "$RESOURCES_DIR/AppIcon.icns"
else
    echo "Warning: Icon file not found at $ICNS_FILE. Run 'make icons' first."
fi

echo ""
echo "Application bundle created: $APP_DIR"
echo "To run: open $APP_DIR"
