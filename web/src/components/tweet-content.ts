import { LitElement, html, css } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { ThreadTweet, api, VideoClip } from '../api';
import { tailwindStyles } from '../styles/shared';
import './media-browser';
import './media-editor';

const fadeInStyles = css`
  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }
  .media-fade-in {
    animation: fadeIn 0.3s ease-out;
  }
`;

interface MediaUrl {
  url: string;
  content_type: string;
}

type VideoOrientation = 'horizontal' | 'vertical' | 'square';
type ImageOrientation = 'horizontal' | 'vertical' | 'square';

interface MediaChoice {
  image_capture_ids: number[];
  video_clip: VideoClip | null;
}

@customElement('tweet-content')
export class TweetContent extends LitElement {
  static styles = [tailwindStyles, fadeInStyles];

  @property({ type: Object }) tweet: ThreadTweet | null = null;
  @property({ type: Boolean }) compact = false;
  @property({ type: Boolean }) showRationale = true;

  @state() imageUrls: MediaUrl[] = [];
  @state() videoUrl: MediaUrl | null = null;
  @state() videoOrientation: VideoOrientation = 'horizontal';
  @state() imageOrientation: ImageOrientation = 'square';
  @state() loadingMedia = true;
  @state() mediaError: string | null = null;
  @state() mediaBrowserOpen = false;
  @state() fullscreenImage: string | null = null;
  @state() editorOpen = false;
  @state() editorCaptureId: number | null = null;
  @state() editorMediaType: 'image' | 'video' = 'image';
  @state() selectedMediaIndex = 0;
  @state() mediaChoices: MediaChoice[] = [];
  @state() private editing = false;
  @state() private editText = '';
  @state() private saving = false;
  @state() private regenerating = false;
  private lastTweetId: number | null = null;
  private lastMediaKey: string | null = null;

  async connectedCallback() {
    super.connectedCallback();
    await this.loadMedia();
  }

  protected updated(changedProperties: Map<string, unknown>) {
    super.updated(changedProperties);
    if (!changedProperties.has('tweet')) {
      return;
    }

    if (!this.tweet) {
      this.mediaChoices = [];
      this.selectedMediaIndex = 0;
      this.lastTweetId = null;
      this.lastMediaKey = null;
      return;
    }

    const tweetId = this.tweet.id;
    const mediaKey = JSON.stringify({
      imageIds: this.tweet.image_capture_ids,
      videoClip: this.tweet.video_clip,
    });

    const choices = this.buildMediaChoices(this.tweet);
    const selectedIndex = this.findMediaChoiceIndex(choices, this.tweet);
    this.mediaChoices = choices;
    this.selectedMediaIndex = selectedIndex === -1 ? 0 : selectedIndex;

    if (tweetId !== this.lastTweetId) {
      this.lastTweetId = tweetId;
    }

    if (mediaKey !== this.lastMediaKey) {
      this.lastMediaKey = mediaKey;
      this.loadMedia();
    }
  }

  async loadMedia() {
    if (!this.tweet) return;

    console.log('[tweet-content] loadMedia - loading capture IDs:', this.tweet.image_capture_ids);
    this.loadingMedia = true;
    this.mediaError = null;
    try {
      // Load images
      const imagePromises = this.tweet.image_capture_ids.map((id) =>
        api.getCaptureUrl(id)
      );
      this.imageUrls = await Promise.all(imagePromises);
      console.log('[tweet-content] loadMedia - got URLs:', this.imageUrls.map(u => u.url));

      // Load video or clear it
      if (this.tweet.video_clip) {
        this.videoUrl = await api.getCaptureUrl(
          this.tweet.video_clip.source_capture_id
        );
        // Detect video orientation by loading metadata
        this.videoOrientation = await this.detectVideoOrientation(this.videoUrl.url);
        this.imageOrientation = 'square';
      } else {
        this.videoUrl = null;
        if (this.imageUrls.length === 1) {
          this.imageOrientation = await this.detectImageOrientation(this.imageUrls[0].url);
        } else {
          this.imageOrientation = 'square';
        }
      }
    } catch (e) {
      console.error('Failed to load media:', e);
      this.mediaError = e instanceof Error ? e.message : 'Failed to load media';
    } finally {
      this.loadingMedia = false;
    }
  }

  private buildMediaChoices(tweet: ThreadTweet): MediaChoice[] {
    const choices: MediaChoice[] = [
      {
        image_capture_ids: tweet.image_capture_ids,
        video_clip: tweet.video_clip,
      },
    ];

    const options = Array.isArray(tweet.media_options) ? tweet.media_options : [];
    for (const option of options) {
      const choice = this.mediaOptionToChoice(option);
      if (choice) {
        choices.push(choice);
      }
    }

    return choices;
  }

  private mediaOptionToChoice(option: unknown): MediaChoice | null {
    if (!option || typeof option !== 'object') return null;

    const record = option as Record<string, unknown>;
    const rawImageIds = record.image_capture_ids;
    const imageIds = Array.isArray(rawImageIds)
      ? rawImageIds.filter((id) => typeof id === 'number') as number[]
      : [];

    const videoCaptureId = typeof record.video_capture_id === 'number'
      ? record.video_capture_id
      : (record.video_clip && typeof record.video_clip === 'object'
        ? (record.video_clip as Record<string, unknown>).source_capture_id
        : null);

    const startTimestamp = typeof record.video_timestamp === 'string'
      ? record.video_timestamp
      : (record.video_clip && typeof record.video_clip === 'object'
        ? (record.video_clip as Record<string, unknown>).start_timestamp
        : null);

    const duration = typeof record.video_duration === 'number'
      ? record.video_duration
      : (record.video_clip && typeof record.video_clip === 'object'
        ? (record.video_clip as Record<string, unknown>).duration_secs
        : null);

    if (typeof videoCaptureId === 'number' && typeof startTimestamp === 'string') {
      return {
        image_capture_ids: [],
        video_clip: {
          source_capture_id: videoCaptureId,
          start_timestamp: startTimestamp,
          duration_secs: typeof duration === 'number' ? duration : 10,
        },
      };
    }

    return {
      image_capture_ids: imageIds,
      video_clip: null,
    };
  }

  private findMediaChoiceIndex(choices: MediaChoice[], tweet: ThreadTweet) {
    return choices.findIndex((choice) => {
      const sameImages = choice.image_capture_ids.length === tweet.image_capture_ids.length
        && choice.image_capture_ids.every((id, i) => id === tweet.image_capture_ids[i]);
      const choiceVideo = choice.video_clip;
      const tweetVideo = tweet.video_clip;
      const sameVideo = !choiceVideo && !tweetVideo
        || (choiceVideo && tweetVideo
          && choiceVideo.source_capture_id === tweetVideo.source_capture_id
          && choiceVideo.start_timestamp === tweetVideo.start_timestamp
          && choiceVideo.duration_secs === tweetVideo.duration_secs);
      return sameImages && sameVideo;
    });
  }

  private detectVideoOrientation(url: string): Promise<VideoOrientation> {
    return new Promise((resolve) => {
      const video = document.createElement('video');
      video.preload = 'metadata';
      video.onloadedmetadata = () => {
        const { videoWidth, videoHeight } = video;
        if (videoWidth > videoHeight * 1.2) {
          resolve('horizontal');
        } else if (videoHeight > videoWidth * 1.2) {
          resolve('vertical');
        } else {
          resolve('square');
        }
      };
      video.onerror = () => resolve('horizontal'); // Default fallback
      video.src = url;
    });
  }

  private detectImageOrientation(url: string): Promise<ImageOrientation> {
    return new Promise((resolve) => {
      const img = new Image();
      img.onload = () => {
        const ratio = img.naturalWidth / img.naturalHeight;
        if (ratio > 1.2) {
          resolve('horizontal');
        } else if (ratio < 0.85) {
          resolve('vertical');
        } else {
          resolve('square');
        }
      };
      img.onerror = () => resolve('square');
      img.src = url;
    });
  }

  openMediaBrowser() {
    this.mediaBrowserOpen = true;
  }

  handleMediaBrowserClose() {
    this.mediaBrowserOpen = false;
  }

  private async selectMedia(index: number) {
    if (!this.tweet) return;
    const choice = this.mediaChoices[index];
    if (!choice) return;

    this.selectedMediaIndex = index;
    this.tweet = {
      ...this.tweet,
      image_capture_ids: choice.image_capture_ids,
      video_clip: choice.video_clip,
    };

    try {
      await api.updateTweetCollateral(this.tweet.id, {
        image_capture_ids: choice.image_capture_ids,
        video_clip: choice.video_clip ?? null,
      });
      this.dispatchEvent(new CustomEvent('collateral-updated', {
        detail: {
          imageIds: choice.image_capture_ids,
          videoId: choice.video_clip?.source_capture_id ?? null,
        },
        bubbles: true,
        composed: true,
      }));
    } catch (e) {
      console.error('Failed to update media option:', e);
      this.mediaError = e instanceof Error ? e.message : 'Failed to update media option';
    }
  }

  async handleCollateralUpdated(e: CustomEvent) {
    if (!this.tweet) return;

    const { imageIds, videoId } = e.detail;
    // Update tweet with new media
    this.tweet = {
      ...this.tweet,
      image_capture_ids: imageIds || [],
      video_clip: videoId
        ? { source_capture_id: videoId, start_timestamp: '00:00:00', duration_secs: 60 }
        : null,
    };
    this.mediaBrowserOpen = false;
    await this.loadMedia();
    // Bubble up the event so parent can react
    this.dispatchEvent(new CustomEvent('collateral-updated', {
      detail: { imageIds, videoId },
      bubbles: true,
      composed: true
    }));
  }

  openEditor(captureId: number, mediaType: 'image' | 'video') {
    this.editorCaptureId = captureId;
    this.editorMediaType = mediaType;
    this.editorOpen = true;
  }

  handleEditorClose() {
    this.editorOpen = false;
  }

  async handleEditComplete(e: CustomEvent<{ newCaptureId: number }>) {
    if (!this.tweet) return;
    const { newCaptureId } = e.detail;
    console.log('[tweet-content] handleEditComplete - newCaptureId:', newCaptureId, 'editorCaptureId:', this.editorCaptureId);
    this.editorOpen = false;

    // Replace the edited capture with the new one in the tweet
    if (this.editorMediaType === 'video' && this.tweet.video_clip) {
      // Update video clip with new capture
      await api.updateTweetCollateral(this.tweet.id, {
        image_capture_ids: [],
        video_clip: {
          source_capture_id: newCaptureId,
          start_timestamp: '00:00:00',
          duration_secs: this.tweet.video_clip.duration_secs,
        },
      });
      this.tweet = {
        ...this.tweet,
        video_clip: {
          ...this.tweet.video_clip,
          source_capture_id: newCaptureId,
        },
      };
    } else if (this.editorCaptureId) {
      // Replace the edited image with the new one
      const newImageIds = this.tweet.image_capture_ids.map((id) =>
        id === this.editorCaptureId ? newCaptureId : id
      );
      console.log('[tweet-content] handleEditComplete - old IDs:', this.tweet.image_capture_ids, 'new IDs:', newImageIds);
      await api.updateTweetCollateral(this.tweet.id, {
        image_capture_ids: newImageIds,
        video_clip: null,
      });
      this.tweet = {
        ...this.tweet,
        image_capture_ids: newImageIds,
      };
    }

    console.log('[tweet-content] handleEditComplete - calling loadMedia with IDs:', this.tweet.image_capture_ids);
    await this.loadMedia();
    this.dispatchEvent(new CustomEvent('collateral-updated', {
      detail: {
        imageIds: this.tweet.image_capture_ids,
        videoId: this.tweet.video_clip?.source_capture_id ?? null,
      },
      bubbles: true,
      composed: true,
    }));
  }

  private startEditing() {
    if (!this.tweet) return;
    this.editText = this.tweet.text;
    this.editing = true;
  }

  private cancelEditing() {
    this.editing = false;
    this.editText = '';
  }

  private async saveText() {
    if (!this.tweet || !this.editText.trim()) return;

    this.saving = true;
    try {
      await api.updateTweetCollateral(this.tweet.id, { text: this.editText.trim() });
      this.tweet = { ...this.tweet, text: this.editText.trim() };
      this.editing = false;
      this.editText = '';
      this.dispatchEvent(new CustomEvent('text-updated', {
        detail: { text: this.tweet.text },
        bubbles: true,
        composed: true,
      }));
    } catch (e) {
      console.error('Failed to save text:', e);
    } finally {
      this.saving = false;
    }
  }

  private handleTextInput(e: InputEvent) {
    const textarea = e.target as HTMLTextAreaElement;
    this.editText = textarea.value;
  }

  private async regenerateTweet() {
    if (!this.tweet || this.regenerating) return;

    this.regenerating = true;
    try {
      const result = await api.regenerateTweet(this.tweet.id);
      this.tweet = { ...this.tweet, text: result.text };
      this.dispatchEvent(new CustomEvent('text-updated', {
        detail: { text: result.text },
        bubbles: true,
        composed: true,
      }));
    } catch (e) {
      console.error('Failed to regenerate tweet:', e);
    } finally {
      this.regenerating = false;
    }
  }

  renderMedia() {
    // Wrapper ensures consistent min-height to prevent layout shift
    const wrapperClass = 'mt-2 min-h-96 rounded-lg overflow-hidden bg-base-200 relative w-full';

    if (this.loadingMedia) {
      return html`
        <div class="${wrapperClass} h-96 flex justify-center items-center">
          <span class="loading loading-spinner loading-sm"></span>
        </div>
      `;
    }

    if (this.mediaError) {
      return html`
        <div class="${wrapperClass} p-3 bg-error/10 border border-error/20">
          <div class="flex items-center gap-2 text-error text-xs">
            <svg class="w-5 h-5 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/>
            </svg>
            <span>${this.mediaError}</span>
          </div>
          <button class="btn btn-sm btn-ghost mt-2" @click=${this.loadMedia}>
            Retry
          </button>
        </div>
      `;
    }

    const hasMedia = this.imageUrls.length > 0 || this.videoUrl;
    if (!hasMedia) {
      return html`
        <button
          class="${wrapperClass} h-96 border-2 border-dashed border-base-300 bg-base-200/50
            flex items-center justify-center gap-1.5 text-sm text-base-content/50 hover:border-primary/50 hover:text-primary/70 transition-colors"
          @click=${this.openMediaBrowser}
        >
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 16l4.586-4.586a2 2 0 012.828 0L16 16m-2-2l1.586-1.586a2 2 0 012.828 0L20 14m-6-6h.01M6 20h12a2 2 0 002-2V6a2 2 0 00-2-2H6a2 2 0 00-2 2v12a2 2 0 002 2z" />
          </svg>
          Add media
        </button>
      `;
    }

    // Fixed container - media fits within using object-contain (no reflow between items)
    return html`
      <div class="${wrapperClass} h-96 media-fade-in">
        ${this.videoUrl
          ? html`
              <video
                controls
                class="w-full h-full object-contain bg-black"
                src=${this.videoUrl.url}
              >
                Your browser does not support the video tag.
              </video>
            `
          : this.renderImageGrid()}

        <!-- Action buttons -->
        <div class="absolute top-2 right-2 flex gap-1">
          <!-- Crop/Trim button -->
          ${this.tweet?.video_clip
            ? html`
                <button
                  class="btn btn-circle btn-sm bg-base-100/80 hover:bg-base-100 border-0 shadow-md"
                  @click=${() => this.openEditor(this.tweet!.video_clip!.source_capture_id, 'video')}
                  title="Trim video"
                >
                  <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M14.121 14.121L19 19m-7-7l7-7m-7 7l-2.879 2.879M12 12L9.121 9.121m0 5.758a3 3 0 10-4.243 4.243 3 3 0 004.243-4.243zm0-5.758a3 3 0 10-4.243-4.243 3 3 0 004.243 4.243z" />
                  </svg>
                </button>
              `
            : this.imageUrls.length === 1
              ? html`
                  <button
                    class="btn btn-circle btn-sm bg-base-100/80 hover:bg-base-100 border-0 shadow-md"
                    @click=${() => this.openEditor(this.tweet!.image_capture_ids[0], 'image')}
                    title="Crop image"
                  >
                    <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 16v2a2 2 0 002 2h2M4 8V6a2 2 0 012-2h2m8 16h2a2 2 0 002-2v-2m0-8V6a2 2 0 00-2-2h-2" />
                    </svg>
                  </button>
                `
              : ''}
          <!-- Change media button -->
          <button
            class="btn btn-circle btn-sm bg-base-100/80 hover:bg-base-100 border-0 shadow-md"
            @click=${this.openMediaBrowser}
            title="Change media"
          >
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 16l4.586-4.586a2 2 0 012.828 0L16 16m-2-2l1.586-1.586a2 2 0 012.828 0L20 14m-6-6h.01M6 20h12a2 2 0 002-2V6a2 2 0 00-2-2H6a2 2 0 00-2 2v12a2 2 0 002 2z" />
            </svg>
          </button>
        </div>

        ${this.tweet?.video_clip
          ? html`
              <div class="absolute bottom-2 left-2 text-xs bg-black/70 text-white px-2 py-1 rounded pointer-events-none">
                ${this.tweet.video_clip.start_timestamp} · ${this.tweet.video_clip.duration_secs}s
              </div>
            `
          : ''}

        ${this.imageUrls.length > 1
          ? html`
              <div class="absolute bottom-2 left-2 text-xs bg-black/70 text-white px-2 py-1 rounded pointer-events-none">
                ${this.imageUrls.length} images
              </div>
            `
          : ''}
      </div>
    `;
  }

  openFullscreen(url: string) {
    this.fullscreenImage = url;
  }

  closeFullscreen() {
    this.fullscreenImage = null;
  }

  renderImageGrid() {
    const count = this.imageUrls.length;

    if (count === 1) {
      return html`
        <img
          src=${this.imageUrls[0].url}
          class="w-full h-full object-contain cursor-pointer"
          @click=${() => this.openFullscreen(this.imageUrls[0].url)}
        />
      `;
    }

    if (count === 2) {
      return html`
        <div class="grid grid-rows-2 h-full gap-0.5">
          ${this.imageUrls.map(
            (img) => html`
              <img
                src=${img.url}
                class="w-full h-full object-cover cursor-pointer"
                @click=${() => this.openFullscreen(img.url)}
              />
            `
          )}
        </div>
      `;
    }

    if (count === 3) {
      return html`
        <div class="grid grid-cols-2 h-full gap-0.5">
          <img
            src=${this.imageUrls[0].url}
            class="w-full h-full object-cover row-span-2 cursor-pointer"
            @click=${() => this.openFullscreen(this.imageUrls[0].url)}
          />
          <img
            src=${this.imageUrls[1].url}
            class="w-full h-full object-cover cursor-pointer"
            @click=${() => this.openFullscreen(this.imageUrls[1].url)}
          />
          <img
            src=${this.imageUrls[2].url}
            class="w-full h-full object-cover cursor-pointer"
            @click=${() => this.openFullscreen(this.imageUrls[2].url)}
          />
        </div>
      `;
    }

    // 4+ images: 2x2 grid with overflow indicator
    return html`
      <div class="grid grid-cols-2 grid-rows-2 h-full gap-0.5">
        ${this.imageUrls.slice(0, 4).map(
          (img, i) => html`
            <div class="relative">
              <img
                src=${img.url}
                class="w-full h-full object-cover cursor-pointer"
                @click=${() => this.openFullscreen(img.url)}
              />
              ${i === 3 && count > 4
                ? html`
                    <div class="absolute inset-0 bg-black/60 flex items-center justify-center text-white text-lg font-bold pointer-events-none">
                      +${count - 4}
                    </div>
                  `
                : ''}
            </div>
          `
        )}
      </div>
    `;
  }

  render() {
    if (!this.tweet) {
      return html`<p class="text-base-content/50">No tweet data</p>`;
    }

    const textSize = this.compact ? 'text-sm' : 'text-base';

    const charCount = this.editing ? this.editText.length : this.tweet.text.length;
    const isOverLimit = charCount > 280;

    return html`
      <!-- Tweet text with edit/refresh controls -->
      <div class="relative group">
        ${this.editing ? html`
          <!-- Edit mode -->
          <div class="space-y-2">
            <textarea
              class="textarea textarea-bordered w-full ${textSize} leading-relaxed resize-none ${isOverLimit ? 'textarea-error' : ''}"
              rows="4"
              .value=${this.editText}
              @input=${this.handleTextInput}
              ?disabled=${this.saving}
              placeholder="Tweet text..."
            ></textarea>
            <div class="flex items-center justify-between">
              <span class="text-xs ${isOverLimit ? 'text-error' : 'text-base-content/50'}">
                ${charCount}/280
              </span>
              <div class="flex gap-2">
                <button
                  class="btn btn-sm btn-ghost"
                  @click=${this.cancelEditing}
                  ?disabled=${this.saving}
                >Cancel</button>
                <button
                  class="btn btn-sm btn-primary"
                  @click=${this.saveText}
                  ?disabled=${this.saving || isOverLimit || !this.editText.trim()}
                >
                  ${this.saving ? html`<span class="loading loading-spinner loading-xs"></span>` : 'Save'}
                </button>
              </div>
            </div>
          </div>
        ` : html`
          <!-- Display mode -->
          <p class="${textSize} leading-relaxed whitespace-pre-wrap h-26 overflow-hidden">${this.tweet.text}</p>
          <!-- Edit/Refresh buttons - show on hover -->
          <div class="absolute top-0 right-0 opacity-0 group-hover:opacity-100 transition-opacity flex gap-1">
            <button
              class="btn btn-xs btn-ghost btn-circle"
              @click=${this.regenerateTweet}
              ?disabled=${this.regenerating}
              title="Regenerate tweet"
            >
              ${this.regenerating
                ? html`<span class="loading loading-spinner loading-xs"></span>`
                : html`<svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                  </svg>`}
            </button>
            <button
              class="btn btn-xs btn-ghost btn-circle"
              @click=${this.startEditing}
              title="Edit tweet"
            >
              <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z" />
              </svg>
            </button>
          </div>
        `}
      </div>

      ${this.mediaChoices.length > 1 ? html`
        <div class="flex items-center gap-1 mt-2">
          <span class="text-xs text-base-content/50">Media:</span>
          ${this.mediaChoices.map((_, i) => html`
            <button
              class="w-5 h-5 text-xs rounded ${i === this.selectedMediaIndex
                ? 'bg-primary text-primary-content font-semibold'
                : 'bg-base-200 text-base-content/60 hover:bg-base-300'}"
              @click=${() => this.selectMedia(i)}
            >
              ${i === 0 ? 'A' : i === 1 ? 'B' : 'C'}
            </button>
          `)}
        </div>
      ` : ''}

      <!-- Media -->
      ${this.renderMedia()}

      <!-- Rationale -->
      ${this.showRationale ? html`
        <details class="mt-3 text-xs text-base-content/60">
          <summary class="cursor-pointer hover:text-base-content/80 font-medium">Why this moment?</summary>
          <p class="mt-1.5 pl-2.5 border-l-2 border-primary/30">${this.tweet.rationale}</p>
        </details>
      ` : ''}

      <!-- Media browser modal -->
      <media-browser
        ?open=${this.mediaBrowserOpen}
        .tweetId=${this.tweet.id}
        .currentImageIds=${this.tweet.image_capture_ids}
        .currentVideoId=${this.tweet.video_clip?.source_capture_id ?? null}
        @close=${this.handleMediaBrowserClose}
        @collateral-updated=${this.handleCollateralUpdated}
      ></media-browser>

      <!-- Media editor modal -->
      <media-editor
        .open=${this.editorOpen}
        .captureId=${this.editorCaptureId}
        .mediaType=${this.editorMediaType}
        @close=${this.handleEditorClose}
        @edit-complete=${this.handleEditComplete}
      ></media-editor>

      <!-- Fullscreen image overlay -->
      ${this.fullscreenImage
        ? html`
            <div
              class="fixed inset-0 z-50 bg-black/90 flex items-center justify-center"
              @click=${this.closeFullscreen}
            >
              <button
                class="absolute top-4 right-4 btn btn-circle btn-ghost text-white"
                @click=${this.closeFullscreen}
              >
                <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
              <img
                class="max-w-[95vw] max-h-[95vh] object-contain"
                src=${this.fullscreenImage}
                @click=${(e: Event) => e.stopPropagation()}
              />
            </div>
          `
        : ''}
    `;
  }
}
