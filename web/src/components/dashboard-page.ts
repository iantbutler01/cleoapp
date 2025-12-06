import { LitElement, html } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { api, User, PendingTweet } from '../api';
import './tweet-card';

@customElement('dashboard-page')
export class DashboardPage extends LitElement {
  @state() user: User | null = null;
  @state() tweets: PendingTweet[] = [];
  @state() loading = true;

  createRenderRoot() {
    return this;
  }

  async connectedCallback() {
    super.connectedCallback();
    await this.loadData();
  }

  async loadData() {
    this.loading = true;
    try {
      const [user, tweets] = await Promise.all([api.getMe(), api.getTweets()]);
      this.user = user;
      this.tweets = tweets;
    } catch (e) {
      console.error('Failed to load data:', e);
    } finally {
      this.loading = false;
    }
  }

  handleLogout() {
    api.clearUserId();
    this.dispatchEvent(new CustomEvent('logout'));
  }

  handleTweetAction() {
    this.loadData();
  }

  render() {
    if (this.loading) {
      return html`
        <div class="flex justify-center items-center min-h-screen">
          <span class="loading loading-spinner loading-lg"></span>
        </div>
      `;
    }

    return html`
      <div class="min-h-screen">
        <!-- Navbar -->
        <div class="navbar bg-base-100 shadow-lg">
          <div class="flex-1">
            <a class="btn btn-ghost text-xl">Cleo</a>
          </div>
          <div class="flex-none gap-2">
            <div class="dropdown dropdown-end">
              <div tabindex="0" role="button" class="btn btn-ghost btn-circle avatar placeholder">
                <div class="bg-neutral text-neutral-content w-10 rounded-full">
                  <span>${this.user?.twitter_username?.charAt(0).toUpperCase() || '?'}</span>
                </div>
              </div>
              <ul
                tabindex="0"
                class="menu menu-sm dropdown-content bg-base-100 rounded-box z-[1] mt-3 w-52 p-2 shadow"
              >
                <li class="menu-title">
                  <span>@${this.user?.twitter_username}</span>
                </li>
                <li><a @click=${this.handleLogout}>Logout</a></li>
              </ul>
            </div>
          </div>
        </div>

        <!-- Content -->
        <div class="container mx-auto px-4 py-8 max-w-2xl">
          <div class="flex justify-between items-center mb-6">
            <h1 class="text-2xl font-bold">Pending Tweets</h1>
            <button class="btn btn-outline btn-sm" @click=${this.loadData}>
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
              </svg>
              Refresh
            </button>
          </div>

          ${this.tweets.length === 0
            ? html`
                <div class="card bg-base-100">
                  <div class="card-body items-center text-center">
                    <svg class="w-16 h-16 opacity-50" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1" d="M20 13V6a2 2 0 00-2-2H6a2 2 0 00-2 2v7m16 0v5a2 2 0 01-2 2H6a2 2 0 01-2-2v-5m16 0h-2.586a1 1 0 00-.707.293l-2.414 2.414a1 1 0 01-.707.293h-3.172a1 1 0 01-.707-.293l-2.414-2.414A1 1 0 006.586 13H4" />
                    </svg>
                    <h2 class="card-title mt-4">No pending tweets</h2>
                    <p class="opacity-70">
                      New tweet suggestions will appear here when the AI finds
                      interesting moments in your recordings.
                    </p>
                  </div>
                </div>
              `
            : html`
                <div class="space-y-4">
                  ${this.tweets.map(
                    (tweet) => html`
                      <tweet-card
                        .tweet=${tweet}
                        @tweet-posted=${this.handleTweetAction}
                        @tweet-dismissed=${this.handleTweetAction}
                      ></tweet-card>
                    `
                  )}
                </div>
              `}
        </div>
      </div>
    `;
  }
}
