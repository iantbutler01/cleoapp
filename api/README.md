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

| Method | Path | Description |
|--------|------|-------------|
| GET | `/auth/twitter` | Get Twitter OAuth URL |
| POST | `/auth/twitter/token` | Exchange OAuth code for session |
| GET | `/me` | Get current user |
| GET | `/tweets` | List pending tweets |
| POST | `/tweets/:id/post` | Post a tweet to Twitter |
| DELETE | `/tweets/:id` | Dismiss a pending tweet |
