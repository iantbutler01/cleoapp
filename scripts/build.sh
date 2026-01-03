#!/bin/bash
# Build Cleo.app bundle with proper code signing
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
DAEMON_DIR="$PROJECT_ROOT/daemon"
BUNDLE_PATH="$DAEMON_DIR/target/release/bundle/osx/Cleo.app"
BUNDLE_ID="com.cleo.cleo"

cd "$DAEMON_DIR"

# Default to metal feature for GPU acceleration
FEATURES="${FEATURES:-metal}"

echo "Building release with features: $FEATURES..."
cargo build --release --features "$FEATURES"

echo "Creating app bundle..."
cargo bundle --release --features "$FEATURES"

echo "Signing with bundle identifier: $BUNDLE_ID"
# Use self-signed certificate if available, otherwise ad-hoc
IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null | grep -E "(Developer ID Application|Apple Development)" | head -1 | awk -F'"' '{print $2}')
if [ -n "$IDENTITY" ]; then
    echo "Using certificate: $IDENTITY"
    codesign --force --deep --sign "$IDENTITY" --identifier "$BUNDLE_ID" "$BUNDLE_PATH"
else
    echo "No certificate found, using ad-hoc signing (permissions won't persist across rebuilds)"
    codesign --force --deep --sign - --identifier "$BUNDLE_ID" "$BUNDLE_PATH"
fi

echo ""
echo "Done! Bundle at: $BUNDLE_PATH"
echo ""
codesign -dv "$BUNDLE_PATH" 2>&1 | grep -E "^(Identifier|Signature)="
echo ""
echo "To install: mv \"$BUNDLE_PATH\" /Applications/"
