#!/bin/bash
# Database migration script using sqlx-cli
# Install: cargo install sqlx-cli --no-default-features --features postgres

set -e

cd "$(dirname "$0")/../api"

# Default DATABASE_URL if not set
export DATABASE_URL="${DATABASE_URL:-postgres://cleo:cleo@localhost/cleo}"

# Check if sqlx is installed
if ! command -v sqlx &> /dev/null; then
    echo "sqlx-cli not found. Installing..."
    cargo install sqlx-cli --no-default-features --features postgres
fi

case "${1:-run}" in
    run)
        echo "Running migrations..."
        sqlx migrate run
        ;;
    revert)
        echo "Reverting last migration..."
        sqlx migrate revert
        ;;
    info)
        echo "Migration status..."
        sqlx migrate info
        ;;
    add)
        if [ -z "$2" ]; then
            echo "Usage: $0 add <migration_name>"
            exit 1
        fi
        echo "Creating new migration: $2"
        sqlx migrate add "$2"
        ;;
    *)
        echo "Usage: $0 {run|revert|info|add <name>}"
        exit 1
        ;;
esac
