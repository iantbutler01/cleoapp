import { LitElement, html } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { PendingTweet, api } from '../api';

@customElement('tweet-card')
export class TweetCard extends LitElement {
  @property({ type: Object }) tweet!: PendingTweet;
  @state() posting = false;
  @state() dismissing = false;

  createRenderRoot() {
    return this;
  }

  async handlePost() {
    this.posting = true;
    try {
      await api.postTweet(this.tweet.id);
      this.dispatchEvent(new CustomEvent('tweet-posted', { detail: this.tweet.id }));
    } catch (e) {
      console.error('Failed to post tweet:', e);
    } finally {
      this.posting = false;
    }
  }

  async handleDismiss() {
    this.dismissing = true;
    try {
      await api.dismissTweet(this.tweet.id);
      this.dispatchEvent(new CustomEvent('tweet-dismissed', { detail: this.tweet.id }));
    } catch (e) {
      console.error('Failed to dismiss tweet:', e);
    } finally {
      this.dismissing = false;
    }
  }

  formatDate(dateStr: string) {
    return new Date(dateStr).toLocaleString();
  }

  render() {
    return html`
      <div class="card bg-base-100 shadow-xl">
        <div class="card-body">
          <div class="flex justify-between items-start">
            <span class="badge badge-ghost">${this.formatDate(this.tweet.created_at)}</span>
            ${this.tweet.video_clip
              ? html`<span class="badge badge-secondary">Video Clip</span>`
              : ''}
            ${this.tweet.image_capture_ids.length > 0
              ? html`<span class="badge badge-accent">${this.tweet.image_capture_ids.length} Images</span>`
              : ''}
          </div>

          <div class="mt-4 p-4 bg-base-200 rounded-lg">
            <p class="text-lg whitespace-pre-wrap">${this.tweet.text}</p>
          </div>

          <div class="collapse collapse-arrow bg-base-200 mt-4">
            <input type="checkbox" />
            <div class="collapse-title font-medium">Why this moment?</div>
            <div class="collapse-content">
              <p class="text-sm opacity-70">${this.tweet.rationale}</p>
            </div>
          </div>

          ${this.tweet.video_clip
            ? html`
                <div class="mt-2 text-sm opacity-70">
                  Video: ${this.tweet.video_clip.start_timestamp} -
                  ${this.tweet.video_clip.duration_secs}s
                </div>
              `
            : ''}

          <div class="card-actions justify-end mt-4">
            <button
              class="btn btn-ghost"
              @click=${this.handleDismiss}
              ?disabled=${this.dismissing || this.posting}
            >
              ${this.dismissing
                ? html`<span class="loading loading-spinner loading-sm"></span>`
                : 'Dismiss'}
            </button>
            <button
              class="btn btn-primary"
              @click=${this.handlePost}
              ?disabled=${this.posting || this.dismissing}
            >
              ${this.posting
                ? html`<span class="loading loading-spinner loading-sm"></span>`
                : html`
                    <svg class="w-4 h-4 mr-1" viewBox="0 0 24 24" fill="currentColor">
                      <path d="M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z"/>
                    </svg>
                    Post
                  `}
            </button>
          </div>
        </div>
      </div>
    `;
  }
}
