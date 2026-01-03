#!/bin/bash
# Seed local development environment with sample data
# Run from api/ directory: ./scripts/seed-dev.sh

set -e

SCRIPT_DIR="$(dirname "$0")"
API_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Default local storage path (matches daemon's default)
LOCAL_STORAGE_PATH="${LOCAL_STORAGE_PATH:-/tmp/cleo-captures}"
DATABASE_URL="${DATABASE_URL:-postgres://cleo:cleo@localhost/cleo}"

echo "=== Cleo Dev Seed Script ==="
echo "LOCAL_STORAGE_PATH: $LOCAL_STORAGE_PATH"
echo "DATABASE_URL: $DATABASE_URL"
echo ""

# Create storage directory
echo "[1/5] Creating local storage directory..."
mkdir -p "$LOCAL_STORAGE_PATH"

# Copy fixture media
echo "[2/5] Copying fixture media..."
cp -r "$API_DIR/fixtures/media/"* "$LOCAL_STORAGE_PATH/"
echo "      Copied images to: $LOCAL_STORAGE_PATH/image/user_1/2025-12-12/"
echo "      Copied videos to: $LOCAL_STORAGE_PATH/video/user_1/2025-12-12/ (MP4 for browser compatibility)"

# Run migrations
echo "[3/5] Running database migrations..."
for migration in "$API_DIR/migrations/"*.sql; do
    echo "      Running $(basename "$migration")..."
    psql "$DATABASE_URL" -f "$migration" -q 2>/dev/null || true
done

# Run seed SQL
echo "[4/5] Loading seed data into database..."
psql "$DATABASE_URL" -f "$API_DIR/fixtures/seed.sql"

# Create thumbnails directory
echo "[5/5] Creating thumbnails directory..."
mkdir -p "$LOCAL_STORAGE_PATH/thumbnails/user_1/2025-12-12"

echo ""
echo "=== Seed Complete! ==="
echo ""
echo "Data created:"
echo "  - ~100 captures across 7 days (images + videos)"
echo "  - 3 threads (2 pending, 1 posted) with 12 tweets total"
echo "  - ~15 standalone tweets (mix of pending and posted)"
echo "  - ~200 activity events"
echo "  - 7 agent runs"
echo ""
echo "To run the API with local storage:"
echo "  LOCAL_STORAGE_PATH=$LOCAL_STORAGE_PATH cargo run"
echo ""
echo "Note: Thumbnails will be generated automatically by the"
echo "      background worker when the API starts."
echo ""
echo "Visit http://localhost:5173 and log in with Twitter."
