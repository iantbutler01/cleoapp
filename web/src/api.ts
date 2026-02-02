import { z } from 'zod';

const API_BASE = '/api';

// OAuth state storage key for CSRF protection
const OAUTH_STATE_KEY = 'cleo_oauth_state';

// ============== Zod Schemas for Runtime Validation ==============

const VideoClipSchema = z.object({
  source_capture_id: z.number(),
  start_timestamp: z.string(),
  duration_secs: z.number(),
});

const UserSchema = z.object({
  id: z.number(),
  username: z.string(),
  display_name: z.string().nullable(),
  created_at: z.string(),
});

const PendingTweetSchema = z.object({
  id: z.number(),
  text: z.string(),
  video_clip: VideoClipSchema.nullable(),
  image_capture_ids: z.array(z.number()),
  rationale: z.string(),
  created_at: z.string(),
});

const PostTweetResponseSchema = z.object({
  tweet_id: z.string(),
  text: z.string(),
});

const ThreadStatusSchema = z.enum(['draft', 'posting', 'posted', 'partial_failed']);

const TweetThreadSchema = z.object({
  id: z.number(),
  title: z.string().nullable(),
  status: ThreadStatusSchema,
  created_at: z.string(),
  posted_at: z.string().nullable(),
  first_tweet_id: z.string().nullable(),
});

const ThreadTweetSchema = z.object({
  id: z.number(),
  text: z.string(),
  video_clip: VideoClipSchema.nullable(),
  image_capture_ids: z.array(z.number()),
  rationale: z.string(),
  created_at: z.string(),
  thread_position: z.number().nullable(),
  reply_to_tweet_id: z.string().nullable(),
  posted_at: z.string().nullable(),
  tweet_id: z.string().nullable(),
});

const ThreadWithTweetsSchema = z.object({
  thread: TweetThreadSchema,
  tweets: z.array(ThreadTweetSchema),
});

const CreateThreadResponseSchema = z.object({
  id: z.number(),
  title: z.string().nullable(),
  tweet_count: z.number(),
});

const PostThreadResponseSchema = z.object({
  status: z.string(),
  tweets: z.array(z.object({
    id: z.number(),
    twitter_id: z.string(),
    reply_to: z.string().nullable(),
  })),
});

const CaptureItemSchema = z.object({
  id: z.number(),
  media_type: z.string(),
  content_type: z.string(),
  captured_at: z.string(),
  thumbnail_url: z.string().nullable(),
  thumbnail_ready: z.boolean(),
});

const BrowseCapturesResponseSchema = z.object({
  captures: z.array(CaptureItemSchema),
  total: z.number(),
  has_more: z.boolean(),
});

const CaptureUrlResponseSchema = z.object({
  url: z.string(),
  content_type: z.string(),
});

const SessionResponseSchema = z.object({
  id: z.number(),
  username: z.string(),
});

// Nudges & Personas schemas
const PersonaSchema = z.object({
  id: z.number(),
  name: z.string(),
  slug: z.string(),
  nudges: z.string(),
});

const UserPersonaSchema = z.object({
  id: z.number(),
  name: z.string(),
  nudges: z.string(),
});

const NudgesResponseSchema = z.object({
  nudges: z.string().nullable(),
  selected_persona_id: z.number().nullable(),
});

const AuthUrlResponseSchema = z.object({
  url: z.string(),
});

const ExchangeTokenResponseSchema = z.object({
  username: z.string(),
});

const ApiTokenResponseSchema = z.object({
  api_token: z.string(),
});

const ContentItemSchema = z.discriminatedUnion('type', [
  z.object({ type: z.literal('tweet') }).extend(ThreadTweetSchema.shape),
  z.object({ type: z.literal('thread'), thread: TweetThreadSchema, tweets: z.array(ThreadTweetSchema) }),
]);

const ContentResponseSchema = z.object({
  items: z.array(ContentItemSchema),
  total: z.number(),
  has_more: z.boolean(),
});

// ============== TypeScript Types (inferred from Zod) ==============

export type VideoClip = z.infer<typeof VideoClipSchema>;
export type User = z.infer<typeof UserSchema>;
export type PendingTweet = z.infer<typeof PendingTweetSchema>;
export type PostTweetResponse = z.infer<typeof PostTweetResponseSchema>;
export type ThreadStatus = z.infer<typeof ThreadStatusSchema>;
export type TweetThread = z.infer<typeof TweetThreadSchema>;
export type ThreadTweet = z.infer<typeof ThreadTweetSchema>;
export type ThreadWithTweets = z.infer<typeof ThreadWithTweetsSchema>;
export type CreateThreadResponse = z.infer<typeof CreateThreadResponseSchema>;
export type PostThreadResponse = z.infer<typeof PostThreadResponseSchema>;
export type CaptureItem = z.infer<typeof CaptureItemSchema>;
export type BrowseCapturesResponse = z.infer<typeof BrowseCapturesResponseSchema>;
export type ContentItem = z.infer<typeof ContentItemSchema>;
export type ContentResponse = z.infer<typeof ContentResponseSchema>;
export type Persona = z.infer<typeof PersonaSchema>;
export type UserPersona = z.infer<typeof UserPersonaSchema>;
export type NudgesResponse = z.infer<typeof NudgesResponseSchema>;

// WebSocket publish progress messages
const PublishProgressSchema = z.discriminatedUnion('type', [
  z.object({ type: z.literal('uploading'), segment: z.number(), total: z.number(), percent: z.number() }),
  z.object({ type: z.literal('processing') }),
  z.object({ type: z.literal('posting') }),
  z.object({ type: z.literal('complete'), tweet_id: z.string(), text: z.string() }),
  z.object({ type: z.literal('error'), message: z.string() }),
]);

export type PublishProgress = z.infer<typeof PublishProgressSchema>;

export interface CreateThreadRequest {
  title?: string;
  tweet_ids: number[];
}

export interface BrowseCapturesParams {
  start?: string;
  end?: string;
  type?: string;
  limit?: number;
  offset?: number;
  include_ids?: number[];
}

export interface GetContentParams {
  platform: 'twitter';
  limit?: number;
  offset?: number;
  status?: 'pending' | 'posted';
}

class ApiClient {
  private refreshTimer: number | null = null;
  private refreshPromise: Promise<boolean> | null = null;
  private onUnauthorized: (() => void) | null = null;
  private unauthorizedFired = false;

  // Cache for capture URLs (signed URLs expire in 15 minutes, cache for 10)
  private captureUrlCache = new Map<number, { data: { url: string; content_type: string }; expires: number }>();
  private readonly CAPTURE_URL_CACHE_TTL = 10 * 60 * 1000; // 10 minutes

  constructor() {
    // Start silent refresh timer (every 7 minutes to refresh before 10-min expiry)
    // 7 minutes gives us a 3-minute buffer for network delays
    this.startRefreshTimer();
  }

  /**
   * Set callback for when session is unauthorized (expired and can't refresh)
   */
  setUnauthorizedCallback(callback: () => void) {
    this.onUnauthorized = callback;
    this.unauthorizedFired = false;
  }

  private startRefreshTimer() {
    // Refresh every 7 minutes (access token expires in 10, gives 3 min buffer)
    const REFRESH_INTERVAL = 7 * 60 * 1000;
    this.refreshTimer = window.setInterval(() => {
      this.silentRefresh();
    }, REFRESH_INTERVAL);
  }

  /**
   * Perform silent refresh. If a refresh is already in progress, return the existing promise.
   * This prevents race conditions where multiple 401 responses trigger concurrent refreshes.
   */
  private silentRefresh(): Promise<boolean> {
    // If already refreshing, return the existing promise so all callers wait for same result
    if (this.refreshPromise) {
      return this.refreshPromise;
    }

    this.refreshPromise = this.doRefresh();
    return this.refreshPromise;
  }

  private async doRefresh(): Promise<boolean> {
    try {
      const res = await fetch(`${API_BASE}/auth/refresh`, {
        method: 'POST',
        credentials: 'include',
      });

      if (!res.ok) {
        // Refresh failed - session expired. Only fire callback once.
        if (this.onUnauthorized && !this.unauthorizedFired) {
          this.unauthorizedFired = true;
          this.onUnauthorized();
        }
        return false;
      }

      // Reset the flag on successful refresh
      this.unauthorizedFired = false;
      return true;
    } catch {
      return false;
    } finally {
      // Clear the promise so next refresh can happen
      this.refreshPromise = null;
    }
  }

  /**
   * Wrapper for fetch that handles 401 errors by attempting refresh.
   * Multiple concurrent 401s will all wait for the same refresh operation.
   */
  private async fetchWithAuth(url: string, options: RequestInit = {}): Promise<Response> {
    const opts: RequestInit = {
      ...options,
      credentials: 'include',
      headers: {
        'Content-Type': 'application/json',
        ...options.headers,
      },
    };

    let res = await fetch(url, opts);

    // If unauthorized, try to refresh and retry once
    if (res.status === 401) {
      const refreshed = await this.silentRefresh();
      if (refreshed) {
        res = await fetch(url, opts);
      }
    }

    return res;
  }

  /**
   * Fetch with auth, parse JSON, and validate with Zod schema
   */
  private async fetchJson<T>(
    url: string,
    options: RequestInit,
    errorMessage: string,
    schema: z.ZodType<T>
  ): Promise<T> {
    const res = await this.fetchWithAuth(url, options);
    if (!res.ok) throw new Error(errorMessage);
    const data = await res.json();
    return schema.parse(data);
  }

  /**
   * Fetch with auth and parse JSON without validation (for simple responses)
   */
  private async fetchJsonRaw<T>(url: string, options: RequestInit = {}, errorMessage: string): Promise<T> {
    const res = await this.fetchWithAuth(url, options);
    if (!res.ok) throw new Error(errorMessage);
    return res.json();
  }

  /**
   * Fetch with auth expecting no response body
   */
  private async fetchVoid(url: string, options: RequestInit = {}, errorMessage: string): Promise<void> {
    const res = await this.fetchWithAuth(url, options);
    if (!res.ok) throw new Error(errorMessage);
  }

  async logout(): Promise<void> {
    // Clear timer first to prevent any refresh attempts during logout
    if (this.refreshTimer) {
      clearInterval(this.refreshTimer);
      this.refreshTimer = null;
    }

    // Clear caches
    this.captureUrlCache.clear();

    try {
      await fetch(`${API_BASE}/auth/logout`, {
        method: 'POST',
        credentials: 'include',
      });
    } catch {
      // Ignore logout errors - we're logging out anyway
    }
  }

  async getAuthUrl(): Promise<{ url: string }> {
    const res = await fetch(`${API_BASE}/auth/twitter`);
    if (!res.ok) throw new Error('Failed to get auth URL');
    const data = await res.json();
    const parsed = AuthUrlResponseSchema.parse(data);

    // Extract and store state from URL for CSRF validation on callback
    // The state parameter is in the URL returned by the backend
    try {
      const authUrl = new URL(parsed.url);
      const state = authUrl.searchParams.get('state');
      if (state) {
        sessionStorage.setItem(OAUTH_STATE_KEY, state);
      }
    } catch {
      // If URL parsing fails, proceed without state storage
      // The backend will still validate on its end
    }

    return parsed;
  }

  /**
   * Validate OAuth state parameter against stored value (client-side CSRF check)
   * Note: Server-side validation is the primary CSRF protection (validates against DB).
   * This client check is defense-in-depth - we allow if no stored state since the
   * server will still reject invalid states.
   */
  validateOAuthState(state: string): boolean {
    const storedState = sessionStorage.getItem(OAUTH_STATE_KEY);
    // Clear stored state regardless of match (one-time use)
    sessionStorage.removeItem(OAUTH_STATE_KEY);

    // Server validates state against DB - this is just an early client-side check.
    // Allow if no stored state (e.g., sessionStorage cleared) since server will validate.
    if (!storedState) {
      return true;
    }

    return storedState === state;
  }

  async exchangeToken(code: string, state: string): Promise<{ username: string }> {
    // Validate state matches what we stored before redirect (CSRF protection)
    if (!this.validateOAuthState(state)) {
      throw new Error('OAuth state mismatch - possible CSRF attack');
    }

    const res = await fetch(`${API_BASE}/auth/twitter/token`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      credentials: 'include',
      body: JSON.stringify({ code, state }),
    });
    if (!res.ok) throw new Error('Failed to exchange token');
    const data = await res.json();
    return ExchangeTokenResponseSchema.parse(data);
  }

  /**
   * Check if session is valid and get basic user info
   */
  async checkSession(): Promise<{ id: number; username: string }> {
    return this.fetchJson(`${API_BASE}/auth/me`, {}, 'Failed to check session', SessionResponseSchema);
  }

  /**
   * Get full user info for display
   */
  async getMe(): Promise<User> {
    return this.fetchJson(`${API_BASE}/me`, {}, 'Failed to get user', UserSchema);
  }

  async getTweets(): Promise<PendingTweet[]> {
    return this.fetchJson(`${API_BASE}/tweets`, {}, 'Failed to get tweets', z.array(PendingTweetSchema));
  }

  async postTweet(id: number): Promise<PostTweetResponse> {
    return this.fetchJson(`${API_BASE}/tweets/${id}/publish`, { method: 'POST' }, 'Failed to post tweet', PostTweetResponseSchema);
  }

  /**
   * Post a tweet via WebSocket with progress updates
   * @param id Tweet ID
   * @param onProgress Callback for progress updates
   * @returns Promise that resolves with tweet_id and text on success
   */
  postTweetWithProgress(
    id: number,
    onProgress: (progress: PublishProgress) => void
  ): Promise<PostTweetResponse> {
    return new Promise((resolve, reject) => {
      // Build WebSocket URL - uses cookies for auth
      const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
      const wsUrl = `${wsProtocol}//${window.location.host}${API_BASE}/tweets/${id}/publish/ws`;

      const ws = new WebSocket(wsUrl);

      ws.onopen = () => {
        // Connection opened, publish will start automatically
      };

      ws.onmessage = (event) => {
        try {
          const raw = JSON.parse(event.data);
          const result = PublishProgressSchema.safeParse(raw);

          if (!result.success) {
            console.error('Invalid WebSocket message format:', result.error);
            return;
          }

          const msg = result.data;
          onProgress(msg);

          if (msg.type === 'complete') {
            ws.close();
            resolve({ tweet_id: msg.tweet_id, text: msg.text });
          } else if (msg.type === 'error') {
            ws.close();
            reject(new Error(msg.message));
          }
        } catch (e) {
          console.error('Failed to parse WebSocket message:', e);
        }
      };

      ws.onerror = () => {
        reject(new Error('WebSocket connection failed'));
      };

      ws.onclose = (event) => {
        if (!event.wasClean && event.code !== 1000) {
          reject(new Error('WebSocket closed unexpectedly'));
        }
      };
    });
  }

  async dismissTweet(id: number): Promise<void> {
    return this.fetchVoid(`${API_BASE}/tweets/${id}`, { method: 'DELETE' }, 'Failed to dismiss tweet');
  }

  async getApiToken(): Promise<string | null> {
    return this.fetchJsonRaw(`${API_BASE}/me/token`, {}, 'Failed to get API token');
  }

  async generateApiToken(): Promise<{ api_token: string }> {
    return this.fetchJson(`${API_BASE}/me/token`, { method: 'POST' }, 'Failed to generate API token', ApiTokenResponseSchema);
  }

  async getCaptureUrl(captureId: number): Promise<{ url: string; content_type: string }> {
    // Check cache first
    const cached = this.captureUrlCache.get(captureId);
    if (cached && cached.expires > Date.now()) {
      return cached.data;
    }

    // Fetch fresh URL
    const data = await this.fetchJson(
      `${API_BASE}/captures/${captureId}/url`,
      {},
      'Failed to get capture URL',
      CaptureUrlResponseSchema
    );

    // Cache the result
    this.captureUrlCache.set(captureId, {
      data,
      expires: Date.now() + this.CAPTURE_URL_CACHE_TTL,
    });

    return data;
  }

  async browseCaptures(params: BrowseCapturesParams = {}): Promise<BrowseCapturesResponse> {
    const query = new URLSearchParams();
    if (params.start) query.set('start', params.start);
    if (params.end) query.set('end', params.end);
    if (params.type) query.set('type', params.type);
    if (params.limit) query.set('limit', params.limit.toString());
    if (params.offset) query.set('offset', params.offset.toString());
    if (params.include_ids?.length) query.set('include_ids', params.include_ids.join(','));

    const url = `${API_BASE}/captures/browse${query.toString() ? '?' + query.toString() : ''}`;
    return this.fetchJson(url, {}, 'Failed to browse captures', BrowseCapturesResponseSchema);
  }

  async updateTweetCollateral(
    tweetId: number,
    collateral: { image_capture_ids?: number[]; video_clip?: VideoClip | null }
  ): Promise<void> {
    return this.fetchVoid(
      `${API_BASE}/tweets/${tweetId}/collateral`,
      { method: 'PUT', body: JSON.stringify(collateral) },
      'Failed to update tweet collateral'
    );
  }

  // Thread methods

  async getThreads(): Promise<TweetThread[]> {
    return this.fetchJson(`${API_BASE}/threads`, {}, 'Failed to get threads', z.array(TweetThreadSchema));
  }

  async getThread(threadId: number): Promise<ThreadWithTweets> {
    return this.fetchJson(`${API_BASE}/threads/${threadId}`, {}, 'Failed to get thread', ThreadWithTweetsSchema);
  }

  async createThread(request: CreateThreadRequest): Promise<CreateThreadResponse> {
    return this.fetchJson(
      `${API_BASE}/threads`,
      { method: 'POST', body: JSON.stringify(request) },
      'Failed to create thread',
      CreateThreadResponseSchema
    );
  }

  async updateThread(
    threadId: number,
    updates: { title?: string; tweet_order?: number[] }
  ): Promise<void> {
    return this.fetchVoid(
      `${API_BASE}/threads/${threadId}`,
      { method: 'PUT', body: JSON.stringify(updates) },
      'Failed to update thread'
    );
  }

  async deleteThread(threadId: number): Promise<void> {
    return this.fetchVoid(`${API_BASE}/threads/${threadId}`, { method: 'DELETE' }, 'Failed to delete thread');
  }

  async addTweetToThread(threadId: number, tweetId: number): Promise<void> {
    return this.fetchVoid(
      `${API_BASE}/threads/${threadId}/tweets`,
      { method: 'POST', body: JSON.stringify({ tweet_id: tweetId }) },
      'Failed to add tweet to thread'
    );
  }

  async removeTweetFromThread(threadId: number, tweetId: number): Promise<void> {
    return this.fetchVoid(
      `${API_BASE}/threads/${threadId}/tweets/${tweetId}`,
      { method: 'DELETE' },
      'Failed to remove tweet from thread'
    );
  }

  async postThread(threadId: number): Promise<PostThreadResponse> {
    return this.fetchJson(
      `${API_BASE}/threads/${threadId}/publish`,
      { method: 'POST' },
      'Failed to post thread',
      PostThreadResponseSchema
    );
  }

  // Content endpoint - unified view of all content
  async getContent(params: GetContentParams): Promise<ContentResponse> {
    const query = new URLSearchParams();
    query.set('platform', params.platform);
    if (params.limit) query.set('limit', params.limit.toString());
    if (params.offset) query.set('offset', params.offset.toString());
    if (params.status) query.set('status', params.status);

    return this.fetchJson(`${API_BASE}/content?${query.toString()}`, {}, 'Failed to get content', ContentResponseSchema);
  }

  // Nudges & Personas

  async getPersonas(): Promise<Persona[]> {
    return this.fetchJson(`${API_BASE}/personas`, {}, 'Failed to get personas', z.array(PersonaSchema));
  }

  async getUserPersonas(): Promise<UserPersona[]> {
    return this.fetchJson(`${API_BASE}/me/personas`, {}, 'Failed to get user personas', z.array(UserPersonaSchema));
  }

  async createUserPersona(name: string): Promise<UserPersona> {
    return this.fetchJson(
      `${API_BASE}/me/personas`,
      { method: 'POST', body: JSON.stringify({ name }) },
      'Failed to create persona',
      UserPersonaSchema
    );
  }

  async deleteUserPersona(id: number): Promise<void> {
    return this.fetchVoid(`${API_BASE}/me/personas/${id}`, { method: 'DELETE' }, 'Failed to delete persona');
  }

  async getNudges(): Promise<NudgesResponse> {
    return this.fetchJson(`${API_BASE}/me/nudges`, {}, 'Failed to get nudges', NudgesResponseSchema);
  }

  async updateNudges(nudges: string, selectedPersonaId?: number | null): Promise<NudgesResponse> {
    return this.fetchJson(
      `${API_BASE}/me/nudges`,
      { method: 'PUT', body: JSON.stringify({ nudges, selected_persona_id: selectedPersonaId }) },
      'Failed to update nudges',
      NudgesResponseSchema
    );
  }
}

export const api = new ApiClient();
