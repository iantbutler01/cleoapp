import { LitElement, html } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { PendingTweet, api } from '../api';

interface MediaUrl {
  url: string;
  content_type: string;
}

@customElement('tweet-card')
export class TweetCard extends LitElement {
  @property({ type: Object }) tweet!: PendingTweet;
  @state() posting = false;
  @state() dismissing = false;
  @state() imageUrls: MediaUrl[] = [];
  @state() videoUrl: MediaUrl | null = null;
  @state() loadingMedia = true;

  createRenderRoot() {
    return this;
  }

  async connectedCallback() {
    super.connectedCallback();
    await this.loadMedia();
  }

  async loadMedia() {
    this.loadingMedia = true;
    try {
      // Load image URLs
      const imagePromises = this.tweet.image_capture_ids.map((id) =>
        api.getCaptureUrl(id)
      );
      this.imageUrls = await Promise.all(imagePromises);

      // Load video URL if present
      if (this.tweet.video_clip) {
        this.videoUrl = await api.getCaptureUrl(
          this.tweet.video_clip.source_capture_id
        );
      }
    } catch (e) {
      console.error('Failed to load media:', e);
    } finally {
      this.loadingMedia = false;
    }
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

  renderMedia() {
    if (this.loadingMedia) {
      return html`
        <div class="flex justify-center py-4">
          <span class="loading loading-spinner loading-md"></span>
        </div>
      `;
    }

    const hasMedia = this.imageUrls.length > 0 || this.videoUrl;
    if (!hasMedia) return '';

    return html`
      <div class="mt-4 space-y-3">
        ${this.videoUrl
          ? html`
              <video
                controls
                class="w-full rounded-lg max-h-80 object-contain bg-black"
                src=${this.videoUrl.url}
              >
                Your browser does not support the video tag.
              </video>
              ${this.tweet.video_clip
                ? html`
                    <div class="text-xs opacity-60">
                      Clip: ${this.tweet.video_clip.start_timestamp} (${this.tweet.video_clip.duration_secs}s)
                    </div>
                  `
                : ''}
            `
          : ''}
        ${this.imageUrls.length > 0
          ? html`
              <div class="grid gap-2 ${this.imageUrls.length > 1 ? 'grid-cols-2' : 'grid-cols-1'}">
                ${this.imageUrls.map(
                  (img) => html`
                    <img
                      src=${img.url}
                      class="rounded-lg w-full object-cover max-h-60 cursor-pointer hover:opacity-90 transition-opacity"
                      @click=${() => window.open(img.url, '_blank')}
                    />
                  `
                )}
              </div>
            `
          : ''}
      </div>
    `;
  }

  render() {
    return html`
      <div class="card bg-base-100 shadow-xl">
        <div class="card-body">
          <div class="flex justify-between items-start">
            <span class="badge badge-ghost">${this.formatDate(this.tweet.created_at)}</span>
            <div class="flex gap-1">
              ${this.tweet.video_clip
                ? html`<span class="badge badge-secondary">Video</span>`
                : ''}
              ${this.tweet.image_capture_ids.length > 0
                ? html`<span class="badge badge-accent">${this.tweet.image_capture_ids.length} Image${this.tweet.image_capture_ids.length > 1 ? 's' : ''}</span>`
                : ''}
            </div>
          </div>

          <div class="mt-4 p-4 bg-base-200 rounded-lg">
            <p class="text-lg whitespace-pre-wrap">${this.tweet.text}</p>
          </div>

          ${this.renderMedia()}

          <div class="collapse collapse-arrow bg-base-200 mt-4">
            <input type="checkbox" />
            <div class="collapse-title font-medium">Why this moment?</div>
            <div class="collapse-content">
              <p class="text-sm opacity-70">${this.tweet.rationale}</p>
            </div>
          </div>

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
