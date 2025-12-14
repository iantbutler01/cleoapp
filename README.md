# Cleo

Cleo is an AI-powered social media assistant that captures your screen activity and suggests tweets based on interesting moments it discovers. A macOS menu bar daemon takes periodic screenshots and automatically records video when it detects activity bursts (rapid mouse clicks or window switches). An AI agent then reviews your captures looking for tweet-worthy moments.

![signal-2025-12-06-155923_002](https://github.com/user-attachments/assets/51d22568-bf91-45db-aea8-ebaa35f33880)

## How It Works

1. **Capture** - A macOS menu bar daemon takes screenshots every 5 seconds and auto-records video during activity bursts
2. **Track** - Mouse clicks and window focus changes are logged to provide context for the AI
3. **Analyze** - An AI agent reviews your captures, looking for interesting moments worth sharing
4. **Review** - Suggested tweets appear in your dashboard with the associated media attached
5. **Post** - Approve and post directly to Twitter/X with one click, or dismiss suggestions you don't want

## Architecture

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│  macOS Daemon   │────▶│    Cleo API     │────▶│  Google Cloud   │
│  (menu bar app) │     │   (Rust/Axum)   │     │    Storage      │
└─────────────────┘     └────────┬────────┘     └─────────────────┘
        │                        │
        │ screenshots            │
        │ video clips    ┌───────┴───────┐
        │ activity logs  │               │
        │           ┌────▼────┐    ┌─────▼─────┐
        └──────────▶│  Web UI │    │ AI Agent  │
                    │  (Lit)  │    │ (Gemini)  │
                    └─────────┘    └───────────┘
```

### Components

- **`/daemon`** - macOS menu bar app (Rust + ScreenCaptureKit)
  - Periodic screenshots (every 5 seconds)
  - Auto-recording on activity bursts (5+ events in 5 seconds)
  - Mouse click and window focus tracking
  - Deep link login (`cleo://login/<api_token>`)

- **`/api`** - Backend server (Rust + Axum)
  - Twitter OAuth 2.0 authentication
  - Capture ingestion and GCS storage
  - AI agent orchestration with Gemini
  - Tweet posting to Twitter/X

- **`/web`** - Frontend dashboard (Lit + TailwindCSS + DaisyUI)
  - Twitter login flow
  - Pending tweet suggestions with media preview
  - One-click posting and dismissal
  - API token generation for daemon auth

## Quick Start

### Prerequisites

- macOS 14+ (for the daemon - uses ScreenCaptureKit)
- Rust nightly (for edition 2024)
- Node.js 18+
- PostgreSQL with TimescaleDB extension
- Google Cloud account (for storage + Gemini API)
- Twitter Developer App credentials
- ffmpeg (optional, for video content filtering)

### Environment Variables

Create a `.env` file or export these variables:

```bash
# Database
export DATABASE_URL=postgres://cleo:cleo@localhost/cleo

# Google Cloud
export GOOGLE_APPLICATION_CREDENTIALS=/path/to/service-account.json
export GOOGLE_GEMINI_API_KEY=your_gemini_api_key
export GCS_BUCKET_NAME=your-bucket-name

# Twitter OAuth 2.0
export TWITTER_CLIENT_ID=your_client_id
export TWITTER_CLIENT_SECRET=your_client_secret
export TWITTER_REDIRECT_URI=http://localhost:5173/auth/twitter/callback
```

---

## Running Each Subsystem

### 1. Database Setup

```bash
# Install PostgreSQL with TimescaleDB
brew install postgresql timescaledb

# Start PostgreSQL
brew services start postgresql

# Create database and user
createdb cleo
psql -d cleo -c "CREATE EXTENSION IF NOT EXISTS timescaledb;"

# Run migrations (from api directory)
cd api
sqlx database create
sqlx migrate run
```

### 2. API Server (`/api`)

The Rust/Axum backend handles authentication, capture storage, and AI agent orchestration.

```bash
cd api

# Install Rust nightly (required for edition 2024)
rustup install nightly
rustup override set nightly

# Run the server (defaults to port 3000)
cargo run

# Or specify a port
PORT=3000 cargo run
```

The API will be available at `http://localhost:3000`.

### 3. Web Dashboard (`/web`)

The Lit + TailwindCSS + DaisyUI frontend for reviewing and posting tweets.

```bash
cd web

# Install dependencies
npm install

# Start development server (port 5173)
npm run dev

# Or build for production
npm run build
npm run preview
```

The dashboard will be available at `http://localhost:5173`.

### 4. macOS Daemon (`/daemon`)

The menu bar app that captures screenshots, records video, and tracks activity.

```bash
cd daemon

# Install Rust nightly
rustup install nightly
rustup override set nightly

# Run with CPU inference (slower, but works everywhere)
cargo run

# Run with Metal GPU acceleration (recommended for Apple Silicon)
cargo run --features metal

# Run with Accelerate framework (optimized for macOS)
cargo run --features accelerate
```

**First Run Notes:**
- Grant Screen Recording permission when prompted (System Settings → Privacy & Security → Screen Recording)
- Grant Accessibility permission for mouse/keyboard tracking
- The NSFW model (~350MB) downloads from Hugging Face on first launch
- If ffmpeg is not installed, video content filtering is skipped

### 5. Connect the Daemon

Once all services are running:

1. Visit `http://localhost:5173`
2. Log in with Twitter
3. Click "Generate API Token"
4. Either:
   - Copy the token to `~/.config/cleo.json`:
     ```json
     {
       "api_token": "cleo_your_token_here"
     }
     ```
   - Or click the deep link: `cleo://login/<api_token>`

The daemon will now upload captures to your account.

---

## Development

### Running All Services

For convenience, run each in a separate terminal:

```bash
# Terminal 1 - API
cd api && cargo run

# Terminal 2 - Web
cd web && npm run dev

# Terminal 3 - Daemon
cd daemon && cargo run --features metal
```

### Useful Commands

```bash
# Check API compilation
cd api && cargo check

# Check daemon compilation
cd daemon && cargo check

# Run API tests
cd api && cargo test

# Build web for production
cd web && npm run build
```

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/auth/twitter` | Get Twitter OAuth URL |
| `POST` | `/auth/twitter/token` | Exchange OAuth code |
| `GET` | `/me` | Get current user |
| `GET` | `/tweets` | List pending tweets |
| `POST` | `/tweets/:id/post` | Post to Twitter |
| `DELETE` | `/tweets/:id` | Dismiss suggestion |
| `GET` | `/captures/:id/url` | Get signed media URL |
| `POST` | `/capture` | Upload capture (daemon) |
| `POST` | `/activity` | Log activity (daemon) |

## How the AI Agent Works

The agent runs on a schedule, processing users who have been idle for a configured period:

1. Fetches recent captures (screenshots, video clips) from GCS
2. Uploads media to Gemini's File API for analysis
3. Prompts the model to identify interesting, tweet-worthy moments
4. Generates tweet text with rationale
5. Stores suggestions in the database for user review

The agent looks for moments like:
- Achievements or milestones
- Interesting code or creative work
- Funny or relatable situations
- Learning moments worth sharing

## Daemon Behavior

The daemon runs as a menu bar app with the following behavior:

- **Screenshots**: Captured every 5 seconds (skipped while recording video)
- **Auto-recording**: Triggered when 5+ activity events occur within 5 seconds
- **Activity events**: Mouse clicks and window focus changes
- **Auto-stop**: Recording stops 5 seconds after activity ceases
- **Manual recording**: Can be toggled from the menu bar

## Security

- **Frontend auth**: Session-based with `X-User-Id` header
- **Daemon auth**: Bearer tokens generated per-user (`cleo_` prefix)
- **Media access**: Time-limited signed URLs (15 min expiry)
- **OAuth tokens**: Auto-refreshed on expiry

## License

AGPLv3
