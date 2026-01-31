import { LitElement, html } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { ThreadTweet, api } from '../api';
import { tailwindStyles } from '../styles/shared';
import './media-browser';
import './media-editor';

interface MediaUrl {
  url: string;
  content_type: string;
}

type VideoOrientation = 'horizontal' | 'vertical' | 'square';

@customElement('tweet-content')
export class TweetContent extends LitElement {
  static styles = [tailwindStyles];

  @property({ type: Object }) tweet: ThreadTweet | null = null;
  @property({ type: Boolean }) compact = false;
  @property({ type: Boolean }) showRationale = true;

  @state() imageUrls: MediaUrl[] = [];
  @state() videoUrl: MediaUrl | null = null;
  @state() videoOrientation: VideoOrientation = 'horizontal';
  @state() loadingMedia = true;
  @state() mediaError: string | null = null;
  @state() mediaBrowserOpen = false;
  @state() fullscreenImage: string | null = null;
  @state() editorOpen = false;
  @state() editorCaptureId: number | null = null;
  @state() editorMediaType: 'image' | 'video' = 'image';

  async connectedCallback() {
    super.connectedCallback();
    await this.loadMedia();
  }

  async loadMedia() {
    if (!this.tweet) return;

    this.loadingMedia = true;
    this.mediaError = null;
    try {
      // Load images
      const imagePromises = this.tweet.image_capture_ids.map((id) =>
        api.getCaptureUrl(id)
      );
      this.imageUrls = await Promise.all(imagePromises);

      // Load video or clear it
      if (this.tweet.video_clip) {
        this.videoUrl = await api.getCaptureUrl(
          this.tweet.video_clip.source_capture_id
        );
        // Detect video orientation by loading metadata
        this.videoOrientation = await this.detectVideoOrientation(this.videoUrl.url);
      } else {
        this.videoUrl = null;
      }
    } catch (e) {
      console.error('Failed to load media:', e);
      this.mediaError = e instanceof Error ? e.message : 'Failed to load media';
    } finally {
      this.loadingMedia = false;
    }
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

  openMediaBrowser() {
    this.mediaBrowserOpen = true;
  }

  handleMediaBrowserClose() {
    this.mediaBrowserOpen = false;
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
      await api.updateTweetCollateral(this.tweet.id, {
        image_capture_ids: newImageIds,
        video_clip: null,
      });
      this.tweet = {
        ...this.tweet,
        image_capture_ids: newImageIds,
      };
    }

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

  renderMedia() {
    if (this.loadingMedia) {
      return html`
        <div class="h-48 flex justify-center items-center bg-base-200 rounded-lg mt-3">
          <span class="loading loading-spinner loading-md"></span>
        </div>
      `;
    }

    if (this.mediaError) {
      return html`
        <div class="mt-3 p-4 rounded-lg bg-error/10 border border-error/20">
          <div class="flex items-center gap-2 text-error text-sm">
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
          class="mt-3 w-full aspect-square max-w-lg mx-auto rounded-2xl border-2 border-dashed border-base-300 bg-base-200/50
            flex items-center justify-center gap-2 text-base-content/50 hover:border-primary/50 hover:text-primary/70 transition-colors"
          @click=${this.openMediaBrowser}
        >
          <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 16l4.586-4.586a2 2 0 012.828 0L16 16m-2-2l1.586-1.586a2 2 0 012.828 0L20 14m-6-6h.01M6 20h12a2 2 0 002-2V6a2 2 0 00-2-2H6a2 2 0 00-2 2v12a2 2 0 002 2z" />
          </svg>
          Add media
        </button>
      `;
    }

    // Adaptive aspect ratio: square for images, orientation-based for video
    const aspectClass = this.videoUrl
      ? this.videoOrientation === 'vertical'
        ? 'aspect-[9/16]'
        : this.videoOrientation === 'square'
        ? 'aspect-square'
        : 'aspect-video'
      : 'aspect-square';

    return html`
      <div class="mt-3 ${aspectClass} max-w-lg mx-auto rounded-2xl overflow-hidden bg-base-200 relative">
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
          class="w-full h-full object-cover cursor-pointer"
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

    const textSize = this.compact ? 'text-base' : 'text-xl';

    return html`
      <!-- Tweet text - no container, directly on card -->
      <p class="${textSize} leading-relaxed whitespace-pre-wrap">${this.tweet.text}</p>

      <!-- Media -->
      ${this.renderMedia()}

      <!-- Rationale -->
      ${this.showRationale ? html`
        <details class="mt-4 text-sm text-base-content/60">
          <summary class="cursor-pointer hover:text-base-content/80 font-medium">Why this moment?</summary>
          <p class="mt-2 pl-3 border-l-2 border-primary/30">${this.tweet.rationale}</p>
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
