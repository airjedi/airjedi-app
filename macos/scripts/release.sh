#!/usr/bin/env bash
# Create a GitHub release with the macOS .app bundle and update the Homebrew tap.
#
# Usage:
#   ./scripts/release.sh              # release current Cargo.toml version
#   ./scripts/release.sh --dry-run    # show what would happen without doing it
#
# Prerequisites: gh (GitHub CLI), ditto, shasum

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MACOS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ROOT_DIR="$(cd "$MACOS_DIR/.." && pwd)"

APP_NAME="AirJedi"
BUILD_DIR="$MACOS_DIR/build"
REPO="airjedi/airjedi-app"
TAP_REPO="airjedi/homebrew-tap"

DRY_RUN=false
if [[ "${1:-}" == "--dry-run" ]]; then
    DRY_RUN=true
fi

# Extract version from Cargo.toml
VERSION=$(grep '^version' "$ROOT_DIR/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')
TAG="v$VERSION"
ZIP_NAME="$APP_NAME-$VERSION-macos-universal.zip"

echo "=== AirJedi Release $TAG ==="
echo ""

# Check for uncommitted changes
if ! git -C "$ROOT_DIR" diff --quiet HEAD 2>/dev/null; then
    echo "Error: uncommitted changes in working tree. Commit or stash first."
    exit 1
fi

# Check tag doesn't already exist as a release
if gh release view "$TAG" -R "$REPO" &>/dev/null; then
    echo "Error: release $TAG already exists on GitHub."
    echo "Bump the version in Cargo.toml first."
    exit 1
fi

echo "Version:  $VERSION"
echo "Tag:      $TAG"
echo "Artifact: $ZIP_NAME"
echo ""

if $DRY_RUN; then
    echo "[dry-run] Would build universal .app, create release $TAG, and update tap."
    exit 0
fi

# Step 1: Build the universal .app bundle
echo "Step 1: Building universal .app bundle..."
(cd "$MACOS_DIR" && make app)
echo ""

# Step 2: Create the zip artifact
echo "Step 2: Creating zip artifact..."
rm -f "$BUILD_DIR/$ZIP_NAME"
(cd "$BUILD_DIR" && ditto -c -k --keepParent "$APP_NAME.app" "$ZIP_NAME")
echo "  Created $BUILD_DIR/$ZIP_NAME ($(du -h "$BUILD_DIR/$ZIP_NAME" | cut -f1))"
echo ""

# Step 3: Compute SHA256
SHA256=$(shasum -a 256 "$BUILD_DIR/$ZIP_NAME" | awk '{print $1}')
echo "  SHA256: $SHA256"
echo ""

# Step 4: Create GitHub release
echo "Step 3: Creating GitHub release $TAG..."
gh release create "$TAG" \
    --repo "$REPO" \
    --title "$APP_NAME $TAG" \
    --generate-notes \
    "$BUILD_DIR/$ZIP_NAME"
echo ""

# Step 5: Update Homebrew tap
echo "Step 4: Updating Homebrew tap..."
TAP_DIR=$(mktemp -d)
gh repo clone "$TAP_REPO" "$TAP_DIR" -- --depth 1 2>/dev/null

CASK_FILE="$TAP_DIR/Casks/airjedi.rb"
if [ ! -f "$CASK_FILE" ]; then
    echo "Error: Cask file not found at $CASK_FILE"
    rm -rf "$TAP_DIR"
    exit 1
fi

# Update version and sha256 in the cask
sed -i '' "s/version \".*\"/version \"$VERSION\"/" "$CASK_FILE"
sed -i '' "s/sha256 \".*\"/sha256 \"$SHA256\"/" "$CASK_FILE"

(cd "$TAP_DIR" && git add -A && git commit -m "Update AirJedi to $VERSION" && git push)
rm -rf "$TAP_DIR"

echo ""
echo "=== Release $TAG complete ==="
echo ""
echo "Install:  brew tap airjedi/tap && brew install --cask airjedi"
echo "Upgrade:  brew upgrade --cask airjedi"
