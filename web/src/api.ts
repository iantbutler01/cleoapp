const API_BASE = '/api';

export interface User {
  id: number;
  twitter_id: string;
  twitter_username: string;
  twitter_name: string | null;
  created_at: string;
}

export interface PendingTweet {
  id: number;
  text: string;
  video_clip: {
    source_capture_id: number;
    start_timestamp: string;
    duration_secs: number;
  } | null;
  image_capture_ids: number[];
  rationale: string;
  created_at: string;
}

export interface PostTweetResponse {
  tweet_id: string;
  text: string;
}

class ApiClient {
  private userId: number | null = null;

  constructor() {
    const stored = localStorage.getItem('user_id');
    if (stored) {
      this.userId = parseInt(stored, 10);
    }
  }

  setUserId(id: number) {
    this.userId = id;
    localStorage.setItem('user_id', id.toString());
  }

  getUserId(): number | null {
    return this.userId;
  }

  clearUserId() {
    this.userId = null;
    localStorage.removeItem('user_id');
  }

  private headers(): HeadersInit {
    const h: HeadersInit = {
      'Content-Type': 'application/json',
    };
    if (this.userId) {
      h['X-User-Id'] = this.userId.toString();
    }
    return h;
  }

  async getAuthUrl(): Promise<{ url: string }> {
    const res = await fetch(`${API_BASE}/auth/twitter`);
    if (!res.ok) throw new Error('Failed to get auth URL');
    return res.json();
  }

  async getMe(): Promise<User> {
    const res = await fetch(`${API_BASE}/me`, { headers: this.headers() });
    if (!res.ok) throw new Error('Failed to get user');
    return res.json();
  }

  async getTweets(): Promise<PendingTweet[]> {
    const res = await fetch(`${API_BASE}/tweets`, { headers: this.headers() });
    if (!res.ok) throw new Error('Failed to get tweets');
    return res.json();
  }

  async postTweet(id: number): Promise<PostTweetResponse> {
    const res = await fetch(`${API_BASE}/tweets/${id}/post`, {
      method: 'POST',
      headers: this.headers(),
    });
    if (!res.ok) throw new Error('Failed to post tweet');
    return res.json();
  }

  async dismissTweet(id: number): Promise<void> {
    const res = await fetch(`${API_BASE}/tweets/${id}`, {
      method: 'DELETE',
      headers: this.headers(),
    });
    if (!res.ok) throw new Error('Failed to dismiss tweet');
  }
}

export const api = new ApiClient();
