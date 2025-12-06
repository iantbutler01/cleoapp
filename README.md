# Cleo

Cleo is an AI-powered social media assistant that captures your screen activity and suggests tweets based on interesting moments it discovers. A macOS menu bar daemon takes periodic screenshots and automatically records video when it detects activity bursts (rapid mouse clicks or window switches). An AI agent then reviews your captures looking for tweet-worthy moments.

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

- macOS (for the daemon - uses ScreenCaptureKit)
- Rust (latest stable)
- Node.js 18+
- PostgreSQL with TimescaleDB extension
- Google Cloud account (for storage + Gemini API)
- Twitter Developer App credentials

### Environment Variables

```bash
# Database
export DATABASE_URL=postgres://cleo:cleo@localhost/cleo

# Google Cloud
export GOOGLE_APPLICATION_CREDENTIALS=/path/to/service-account.json
export GOOGLE_GEMINI_API_KEY=your_gemini_api_key

# Twitter OAuth 2.0
export TWITTER_CLIENT_ID=your_client_id
export TWITTER_CLIENT_SECRET=your_client_secret
export TWITTER_REDIRECT_URI=http://localhost:5173/auth/twitter/callback
```

### Running Locally

**Start the API:**
```bash
cd api
cargo run
```

**Start the Web UI:**
```bash
cd web
npm install
npm run dev
```

**Start the Daemon:**
```bash
cd daemon
cargo run
```

Visit `http://localhost:5173` to log in with Twitter, generate an API token, and configure the daemon.

### Daemon Configuration

The daemon reads its API token from `~/.config/cleo.json`:

```json
{
  "api_token": "cleo_your_token_here"
}
```

You can also configure it via deep link: `cleo://login/<api_token>`

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

MIT
