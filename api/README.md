# Cleo API

Rust API server using Axum, TimescaleDB, and Twitter OAuth 2.0.

## Prerequisites

- Rust (latest stable)
- PostgreSQL with TimescaleDB extension
- Twitter Developer App credentials

## Environment Variables

```bash
export DATABASE_URL=postgres://cleo:cleo@localhost/cleo
export TWITTER_CLIENT_ID=your_client_id
export TWITTER_CLIENT_SECRET=your_client_secret
export TWITTER_REDIRECT_URI=http://localhost:5173/auth/twitter/callback
export PORT=3000  # optional, defaults to 3000
```

## Database Setup

Run migrations against your TimescaleDB instance:

```bash
psql $DATABASE_URL -f migrations/001_init.sql
psql $DATABASE_URL -f migrations/002_users.sql
```

## Running

```bash
cargo run
```

The API will be available at `http://localhost:3000`.

## Endpoints

### Authentication
The API uses two auth mechanisms:
- **X-User-Id header**: For frontend session auth (used by web UI)
- **Bearer token**: For daemon auth (used by capture daemon)

| Method | Path | Description | Auth |
|--------|------|-------------|------|
| GET | `/auth/twitter` | Get Twitter OAuth URL | None |
| POST | `/auth/twitter/token` | Exchange OAuth code for session | None |
| GET | `/me` | Get current user | X-User-Id |
| GET | `/me/token` | Get current API token | X-User-Id |
| POST | `/me/token` | Generate new API token | X-User-Id |
| GET | `/tweets` | List pending tweets | X-User-Id |
| POST | `/tweets/:id/post` | Post a tweet to Twitter | X-User-Id |
| DELETE | `/tweets/:id` | Dismiss a pending tweet | X-User-Id |
| POST | `/capture` | Upload screen capture | Bearer |
| POST | `/activity` | Log user activity | Bearer |

## Daemon Authentication

The capture daemon uses Bearer token authentication. Users generate an API token from the web UI, then configure the daemon with:

```bash
export CLEO_API_TOKEN=cleo_xxxxxxxxxxxxx
```

The daemon sends this token in the Authorization header:
```
Authorization: Bearer cleo_xxxxxxxxxxxxx
```
