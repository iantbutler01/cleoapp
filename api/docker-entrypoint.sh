#!/bin/bash
set -e

# Run migrations if DATABASE_URL is set
if [ -n "$DATABASE_URL" ]; then
    echo "Running database migrations..."

    for migration in /app/migrations/*.sql; do
        if [ -f "$migration" ]; then
            echo "Applying $(basename "$migration")..."
            psql "$DATABASE_URL" -f "$migration" 2>/dev/null || echo "  (already applied or skipped)"
        fi
    done

    echo "Migrations complete."
fi

exec "$@"
