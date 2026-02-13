# Cleo Helm Deployment Notes (AI Handoff)

This is the deployment handoff for the chart at `helm/cleo`.

## Chart overview

- Chart path: `helm/cleo`
- Deploys:
  - API `Deployment` + `Service` on port `3000`
  - Web `Deployment` + `Service` on port `4173`
  - Optional `Ingress`
  - Optional `ServiceAccount`
- Images are composed from:
  - `image.registry`
  - `api.image.repository` + `api.image.tag`
  - `web.image.repository` + `web.image.tag`
  - See `helm/cleo/templates/_helpers.tpl`.

## Important chart behavior

1. API env defaults are auto-injected by template if not explicitly set:
   - `APP_ORIGIN`
   - `CORS_ORIGIN`
   - `COOKIE_SAMESITE`
   - `TWITTER_REDIRECT_URI`
   - Logic is in `helm/cleo/templates/api-deployment.yaml`.
2. Web container gets only one env from chart:
   - `API_BASE_URL` (from `web.apiBaseUrl`)
3. Split-domain ingress mode is enabled automatically when either:
   - `ingress.api.hosts` has entries, or
   - `ingress.web.hosts` has entries.
4. In split mode, callback path is auto-added to web paths:
   - `app.twitterCallbackPath` is appended to `ingress.web.paths` if missing.

## Required API runtime env (must be provided)

From `api/src/main.rs`:

- Required:
  - `TWITTER_CLIENT_ID`
  - `TWITTER_CLIENT_SECRET`
  - `JWT_SECRET` (must be at least 32 bytes)
- Strongly recommended / usually required:
  - `DATABASE_URL`
  - `GOOGLE_APPLICATION_CREDENTIALS` or workload identity equivalent for GCS access
- Optional:
  - `GOOGLE_GEMINI_API_KEY` (agent disabled if unset)
  - `LOCAL_STORAGE_PATH`
  - `AGENT_IDLE_MINUTES` (default 20)
  - `AGENT_CHECK_INTERVAL_SECS` (default 300)
  - `DB_POOL_SIZE`
  - `VAPID_PUBLIC_KEY`, `VAPID_PRIVATE_KEY` (web push feature)

## Production cookie requirement

`api/src/services/cookies.rs` treats any `ENV != prod` as dev and omits `Secure` cookie attribute.

Set this for production:

- `ENV=prod`

Also set cross-domain mode correctly:

- If web and API are different origins: `COOKIE_SAMESITE=None`
- If same origin: `COOKIE_SAMESITE=Lax` is fine

## Known limitations / caveats

1. `externalSecrets` block exists in `values.yaml` but is not wired to templates.
2. Only API supports `envFrom`; web does not.
3. No chart values for extra volumes/volumeMounts (for mounting GCP key JSON) out of the box.
4. API startup currently always constructs GCS client at boot, even if `LOCAL_STORAGE_PATH` is set.
5. API bucket name is hardcoded in `api/src/constants.rs` (`cleo_multimedia_data`), not a Helm value.
6. API entrypoint runs SQL files with `psql` and suppresses migration errors as "already applied or skipped"; validate DB state separately.

## Single-domain values example

```yaml
image:
  registry: ghcr.io/your-org

app:
  webOrigin: https://cleo.example.com
  twitterCallbackPath: /auth/twitter/callback
  cookieSameSite: Lax

api:
  image:
    repository: cleo-api
    tag: sha-<gitsha>
  env:
    ENV: prod
    DATABASE_URL: postgres://user:pass@postgres:5432/cleo
    TWITTER_CLIENT_ID: <set>
    TWITTER_CLIENT_SECRET: <set>
    JWT_SECRET: <32+ byte secret>
    GOOGLE_APPLICATION_CREDENTIALS: /var/secrets/google/key.json
    # GOOGLE_GEMINI_API_KEY: <optional>
  # optional:
  # envFrom:
  #   - secretRef:
  #       name: cleo-api-secrets

web:
  image:
    repository: cleo-web
    tag: sha-<gitsha>
  apiBaseUrl: /api

ingress:
  enabled: true
  className: nginx
  hosts:
    - host: cleo.example.com
      paths:
        - path: /api
          pathType: Prefix
          service: api
        - path: /media
          pathType: Prefix
          service: api
        - path: /auth
          pathType: Prefix
          service: api
        - path: /auth/twitter/callback
          pathType: Prefix
          service: web
        - path: /
          pathType: Prefix
          service: web
```

## Split-domain values example

```yaml
image:
  registry: ghcr.io/your-org

app:
  webOrigin: https://app.example.com
  twitterCallbackPath: /auth/twitter/callback
  cookieSameSite: None

api:
  image:
    repository: cleo-api
    tag: sha-<gitsha>
  env:
    ENV: prod
    DATABASE_URL: postgres://user:pass@postgres:5432/cleo
    TWITTER_CLIENT_ID: <set>
    TWITTER_CLIENT_SECRET: <set>
    JWT_SECRET: <32+ byte secret>

web:
  image:
    repository: cleo-web
    tag: sha-<gitsha>
  apiBaseUrl: https://api.example.com

ingress:
  enabled: true
  className: nginx
  api:
    hosts:
      - api.example.com
    paths:
      - /api
      - /media
      - /auth
  web:
    hosts:
      - app.example.com
    paths:
      - /
```

## Deployment commands

```bash
# 1) Render sanity check
helm lint helm/cleo
helm template cleo helm/cleo -f your-values.yaml >/tmp/cleo-render.yaml

# 2) Deploy/upgrade
helm upgrade --install cleo helm/cleo \
  --namespace cleo \
  --create-namespace \
  -f your-values.yaml

# 3) Check rollout
kubectl -n cleo get pods
kubectl -n cleo rollout status deploy/cleo-api
kubectl -n cleo rollout status deploy/cleo-web
kubectl -n cleo get ingress
```

## Post-deploy smoke checks

```bash
# API health
kubectl -n cleo port-forward svc/cleo-api 3000:3000 &
curl -sf http://127.0.0.1:3000/health

# Web
kubectl -n cleo port-forward svc/cleo-web 4173:4173 &
curl -I http://127.0.0.1:4173/
```

