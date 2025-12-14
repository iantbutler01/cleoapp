#!/bin/bash
# Seed local development environment with sample data
# Run from api/ directory: ./scripts/seed-dev.sh

set -e

# Default local storage path
LOCAL_STORAGE_PATH="${LOCAL_STORAGE_PATH:-/tmp/cleo-storage}"

echo "=== Cleo Dev Seed Script ==="
echo "LOCAL_STORAGE_PATH: $LOCAL_STORAGE_PATH"
echo ""

# Create storage directory
echo "[1/3] Creating local storage directory..."
mkdir -p "$LOCAL_STORAGE_PATH"

# Copy fixture media
echo "[2/3] Copying fixture media..."
cp -r "$(dirname "$0")/../fixtures/media/"* "$LOCAL_STORAGE_PATH/"
echo "      Copied to: $LOCAL_STORAGE_PATH/image/user_1/2025-12-12/"

# Run seed SQL
echo "[3/3] Loading seed data into database..."
psql -d cleo -f "$(dirname "$0")/../fixtures/seed.sql"

echo ""
echo "=== Done! ==="
echo ""
echo "To run the API with local storage:"
echo "  LOCAL_STORAGE_PATH=$LOCAL_STORAGE_PATH cargo run"
echo ""
echo "Then visit http://localhost:5173 and log in with Twitter."
