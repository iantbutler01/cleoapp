import { LitElement, html, css } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { repeat } from 'lit/directives/repeat.js';
import { CaptureItem, api } from '../api';
import { tailwindStyles } from '../styles/shared';

@customElement('media-browser')
export class MediaBrowser extends LitElement {
  static styles = [
    tailwindStyles,
    css`
      .media-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(100px, 1fr));
        gap: 8px;
        overflow-y: auto;
        padding: 4px;
        scroll-behavior: smooth;
      }
      .media-grid::-webkit-scrollbar {
        width: 8px;
      }
      .media-grid::-webkit-scrollbar-track {
        background: oklch(var(--b2));
        border-radius: 4px;
      }
      .media-grid::-webkit-scrollbar-thumb {
        background: oklch(var(--bc) / 0.3);
        border-radius: 4px;
      }
      .thumbnail {
        width: 100%;
        aspect-ratio: 4/3;
        object-fit: cover;
        border-radius: 6px;
        cursor: pointer;
        border: 2px solid transparent;
        transition: all 0.15s ease;
        background: oklch(var(--b2));
      }
      .thumbnail:hover {
        border-color: oklch(var(--p) / 0.5);
      }
      .thumbnail.selected {
        border-color: oklch(var(--p));
        box-shadow: 0 0 0 2px oklch(var(--p) / 0.3);
      }
      .thumbnail-placeholder {
        width: 100%;
        aspect-ratio: 4/3;
        border-radius: 6px;
        background: oklch(var(--b2));
        display: flex;
        align-items: center;
        justify-content: center;
      }
    `,
  ];

  @property({ type: Boolean }) open = false;
  @property({ type: Number }) tweetId: number | null = null;
  @property({ type: Array }) currentImageIds: number[] = [];
  @property({ type: Number }) currentVideoId: number | null = null;

  @state() captures: CaptureItem[] = [];
  @state() loading = true;
  @state() loadingMore = false;
  @state() loadError: string | null = null;
  @state() selectedCapture: CaptureItem | null = null;
  @state() selectedIds: number[] = [];
  @state() selectedVideoId: number | null = null;
  @state() previewUrl: string | null = null;
  @state() previewLoading = false;
  @state() previewError: string | null = null;
  @state() hasMore = false;
  @state() total = 0;
  @state() saving = false;
  @state() saveError: string | null = null;
  @state() filterType: string = '';
  @state() fullscreen = false;

  async connectedCallback() {
    super.connectedCallback();
    if (this.open) {
      await this.loadCaptures();
    }
  }

  async updated(changedProperties: Map<string, unknown>) {
    if (changedProperties.has('open') && this.open) {
      this.selectedIds = [...this.currentImageIds];
      this.selectedVideoId = this.currentVideoId;
      await this.loadCaptures();
    }
  }

  async loadCaptures(append = false) {
    if (append) {
      this.loadingMore = true;
    } else {
      this.loading = true;
      this.loadError = null;
      this.captures = [];
    }

    try {
      // On initial load, include selected IDs so they're fetched even if not in first page
      const includeIds = !append
        ? [...this.selectedIds, ...(this.selectedVideoId ? [this.selectedVideoId] : [])]
        : undefined;

      const response = await api.browseCaptures({
        type: this.filterType || undefined,
        limit: 50,
        offset: append ? this.captures.length : 0,
        include_ids: includeIds?.length ? includeIds : undefined,
      });

      if (append) {
        this.captures = [...this.captures, ...response.captures];
      } else {
        this.captures = response.captures;
      }
      this.hasMore = response.has_more;
      this.total = response.total;
    } catch (e) {
      console.error('Failed to load captures:', e);
      this.loadError = e instanceof Error ? e.message : 'Failed to load captures';
    } finally {
      this.loading = false;
      this.loadingMore = false;
    }
  }

  async selectCapture(capture: CaptureItem) {
    this.selectedCapture = capture;
    this.previewLoading = true;
    this.previewUrl = null;
    this.previewError = null;

    try {
      const result = await api.getCaptureUrl(capture.id);
      this.previewUrl = result.url;
    } catch (e) {
      console.error('Failed to load preview:', e);
      this.previewError = e instanceof Error ? e.message : 'Failed to load preview';
    } finally {
      this.previewLoading = false;
    }
  }

  toggleSelection(capture: CaptureItem) {
    if (capture.media_type === 'video') {
      // Video selection - toggle or replace
      if (this.selectedVideoId === capture.id) {
        // Deselect video
        this.selectedVideoId = null;
      } else {
        // Select this video, clear any images
        this.selectedVideoId = capture.id;
        this.selectedIds = [];
      }
    } else {
      // Image selection
      const idx = this.selectedIds.indexOf(capture.id);
      if (idx >= 0) {
        // Deselect image
        this.selectedIds = this.selectedIds.filter((id) => id !== capture.id);
      } else {
        // Select image - clear video if any, max 4 images
        if (this.selectedVideoId !== null) {
          this.selectedVideoId = null;
        }
        if (this.selectedIds.length < 4) {
          this.selectedIds = [...this.selectedIds, capture.id];
        }
      }
    }
    // Force update to ensure thumbnails re-render
    this.requestUpdate();
    this.selectCapture(capture);
  }

  async handleSave() {
    if (!this.tweetId) return;

    this.saving = true;
    this.saveError = null;
    try {
      if (this.selectedVideoId) {
        // Save video
        await api.updateTweetCollateral(this.tweetId, {
          image_capture_ids: [],
          video_clip: {
            source_capture_id: this.selectedVideoId,
            start_timestamp: '00:00:00',
            duration_secs: 60, // TODO: get actual duration
          },
        });
        this.dispatchEvent(
          new CustomEvent('collateral-updated', {
            detail: { tweetId: this.tweetId, imageIds: [], videoId: this.selectedVideoId },
            bubbles: true,
            composed: true,
          })
        );
      } else {
        // Save images
        await api.updateTweetCollateral(this.tweetId, {
          image_capture_ids: this.selectedIds,
          video_clip: null,
        });
        this.dispatchEvent(
          new CustomEvent('collateral-updated', {
            detail: { tweetId: this.tweetId, imageIds: this.selectedIds, videoId: null },
            bubbles: true,
            composed: true,
          })
        );
      }
      this.close();
    } catch (e) {
      console.error('Failed to save collateral:', e);
      this.saveError = e instanceof Error ? e.message : 'Failed to save';
    } finally {
      this.saving = false;
    }
  }

  close() {
    this.open = false;
    this.selectedCapture = null;
    this.previewUrl = null;
    this.dispatchEvent(new CustomEvent('close', { bubbles: true, composed: true }));
  }

  handleFilterChange(e: Event) {
    const target = e.target;
    if (target instanceof HTMLSelectElement) {
      this.filterType = target.value;
      this.loadCaptures();
    }
  }

  formatTime(dateStr: string) {
    return new Date(dateStr).toLocaleString(undefined, {
      month: 'short',
      day: 'numeric',
      hour: 'numeric',
      minute: '2-digit',
    });
  }

  /** Sort captures so selected items appear first */
  getSortedCaptures(): CaptureItem[] {
    const selected: CaptureItem[] = [];
    const unselected: CaptureItem[] = [];

    for (const capture of this.captures) {
      const isSelected = capture.media_type === 'video'
        ? this.selectedVideoId === capture.id
        : this.selectedIds.includes(capture.id);

      if (isSelected) {
        selected.push(capture);
      } else {
        unselected.push(capture);
      }
    }

    // For images, maintain selection order (1st selected, 2nd selected, etc.)
    if (this.selectedVideoId === null && this.selectedIds.length > 0) {
      selected.sort((a, b) =>
        this.selectedIds.indexOf(a.id) - this.selectedIds.indexOf(b.id)
      );
    }

    return [...selected, ...unselected];
  }

  render() {
    if (!this.open) return html``;

    return html`
      <div class="modal modal-open">
        <div class="modal-box max-w-4xl h-[80vh] flex flex-col">
          <div class="flex justify-between items-center mb-4">
            <h3 class="font-bold text-lg">Select Media</h3>
            <button class="btn btn-sm btn-circle btn-ghost" @click=${this.close}>✕</button>
          </div>

          <!-- Filter -->
          <div class="flex gap-2 mb-4">
            <select class="select select-bordered select-sm" @change=${this.handleFilterChange}>
              <option value="">All Media</option>
              <option value="image">Images Only</option>
              <option value="video">Videos Only</option>
            </select>
            <span class="text-sm opacity-60 self-center">${this.total} captures</span>
          </div>

          <!-- Media Grid -->
          <div class="flex-1 min-h-0">
            ${this.loading
              ? html`
                  <div class="flex justify-center py-8">
                    <span class="loading loading-spinner loading-md"></span>
                  </div>
                `
              : this.loadError
              ? html`
                  <div class="flex flex-col items-center justify-center py-8 text-center">
                    <svg class="w-10 h-10 text-error mb-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/>
                    </svg>
                    <p class="text-error text-sm">${this.loadError}</p>
                    <button class="btn btn-sm btn-ghost mt-2" @click=${() => this.loadCaptures()}>Retry</button>
                  </div>
                `
              : html`
                  <div class="media-grid h-full">
                    ${repeat(
                      this.getSortedCaptures(),
                      (capture) => capture.id,
                      (capture) => this.renderThumbnail(capture)
                    )}
                    ${this.hasMore
                      ? html`
                          <button
                            class="btn btn-ghost btn-sm aspect-4/3 w-full"
                            @click=${() => this.loadCaptures(true)}
                            ?disabled=${this.loadingMore}
                          >
                            ${this.loadingMore
                              ? html`<span class="loading loading-spinner loading-sm"></span>`
                              : 'Load More'}
                          </button>
                        `
                      : ''}
                  </div>
                `}
          </div>

          <!-- Preview Area -->
          <div class="flex-1 flex flex-col min-h-0">
            ${this.selectedCapture
              ? html`
                  <div class="relative flex-1 flex items-center justify-center bg-base-200 rounded-lg overflow-hidden">
                    ${this.previewLoading
                      ? html`<span class="loading loading-spinner loading-lg"></span>`
                      : this.previewError
                        ? html`
                            <div class="text-center">
                              <span class="text-error text-sm">${this.previewError}</span>
                              <button class="btn btn-xs btn-ghost mt-1" @click=${() => {
                                if (this.selectedCapture) this.selectCapture(this.selectedCapture);
                              }}>Retry</button>
                            </div>
                          `
                        : this.previewUrl
                          ? this.selectedCapture.media_type === 'video'
                            ? html`
                                <video
                                  controls
                                  class="max-w-full max-h-full object-contain"
                                  src=${this.previewUrl}
                                ></video>
                              `
                            : html`
                                <img
                                  class="max-w-full max-h-full object-contain"
                                  src=${this.previewUrl}
                                />
                              `
                          : html`<span class="text-error">No preview available</span>`}
                    ${this.previewUrl && !this.previewLoading
                      ? html`
                          <button
                            class="absolute top-2 right-2 btn btn-circle btn-sm btn-ghost bg-base-100/80"
                            @click=${() => (this.fullscreen = true)}
                            title="View fullscreen"
                          >
                            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 8V4m0 0h4M4 4l5 5m11-1V4m0 0h-4m4 0l-5 5M4 16v4m0 0h4m-4 0l5-5m11 5l-5-5m5 5v-4m0 4h-4" />
                            </svg>
                          </button>
                        `
                      : ''}
                  </div>
                  <div class="mt-2 text-sm opacity-60 text-center">
                    ${this.formatTime(this.selectedCapture.captured_at)} - ${this.selectedCapture.media_type}
                  </div>
                `
              : html`
                  <div class="flex-1 flex items-center justify-center text-base-content/50">
                    Select a capture to preview
                  </div>
                `}
          </div>

          <!-- Fullscreen Overlay -->
          ${this.fullscreen && this.previewUrl && this.selectedCapture
            ? html`
                <div
                  class="fixed inset-0 z-50 bg-black/90 flex items-center justify-center"
                  @click=${(e: Event) => {
                    e.stopPropagation();
                    this.fullscreen = false;
                  }}
                >
                  <button
                    class="absolute top-4 right-4 btn btn-circle btn-ghost text-white"
                    @click=${(e: Event) => {
                      e.stopPropagation();
                      this.fullscreen = false;
                    }}
                  >
                    <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
                    </svg>
                  </button>
                  ${this.selectedCapture.media_type === 'video'
                    ? html`
                        <video
                          controls
                          autoplay
                          class="max-w-[95vw] max-h-[95vh] object-contain"
                          src=${this.previewUrl}
                          @click=${(e: Event) => e.stopPropagation()}
                        ></video>
                      `
                    : html`
                        <img
                          class="max-w-[95vw] max-h-[95vh] object-contain"
                          src=${this.previewUrl}
                          @click=${(e: Event) => e.stopPropagation()}
                        />
                      `}
                </div>
              `
            : ''}

          <!-- Selection Info & Actions -->
          <div class="mt-4">
            ${this.saveError
              ? html`
                  <div class="alert alert-error mb-3 py-2">
                    <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/>
                    </svg>
                    <span class="text-sm">${this.saveError}</span>
                  </div>
                `
              : ''}
            <div class="flex justify-between items-center">
              <div class="text-sm">
                ${this.selectedVideoId
                  ? html`<span class="badge badge-primary">1 video selected</span>`
                  : this.selectedIds.length > 0
                    ? html`<span class="badge badge-primary">${this.selectedIds.length} image${this.selectedIds.length > 1 ? 's' : ''} selected</span>`
                    : html`<span class="opacity-60">Select 1 video or up to 4 images</span>`}
              </div>
              <div class="flex gap-2">
                <button class="btn btn-ghost" @click=${this.close}>Cancel</button>
                <button
                  class="btn btn-primary"
                  @click=${this.handleSave}
                  ?disabled=${this.saving || (this.selectedIds.length === 0 && !this.selectedVideoId)}
                >
                  ${this.saving
                    ? html`<span class="loading loading-spinner loading-sm"></span>`
                    : this.selectedVideoId
                      ? 'Save Video'
                      : `Save (${this.selectedIds.length})`}
                </button>
              </div>
            </div>
          </div>
        </div>
        <div class="modal-backdrop bg-black/50" @click=${(e: Event) => {
          if (this.fullscreen) {
            e.stopPropagation();
            this.fullscreen = false;
          } else {
            this.close();
          }
        }}></div>
      </div>
    `;
  }

  renderThumbnail(capture: CaptureItem) {
    const isVideo = capture.media_type === 'video';
    const isSelected = isVideo
      ? this.selectedVideoId === capture.id
      : this.selectedIds.includes(capture.id);
    const isActive = this.selectedCapture?.id === capture.id;

    if (!capture.thumbnail_ready || !capture.thumbnail_url) {
      return html`
        <div
          class="thumbnail-placeholder cursor-pointer"
          @click=${() => this.toggleSelection(capture)}
          title="Thumbnail generating..."
        >
          <span class="loading loading-spinner loading-xs"></span>
        </div>
      `;
    }

    return html`
      <div class="relative">
        <img
          class="thumbnail ${isSelected ? 'selected' : ''} ${isActive ? 'ring-2 ring-offset-2 ring-primary' : ''}"
          src=${capture.thumbnail_url}
          @click=${() => this.toggleSelection(capture)}
          title=${this.formatTime(capture.captured_at)}
        />
        ${isSelected
          ? html`
              <div class="absolute top-1 right-1 badge badge-primary badge-xs">
                ${isVideo ? '✓' : this.selectedIds.indexOf(capture.id) + 1}
              </div>
            `
          : ''}
        ${isVideo
          ? html`
              <div class="absolute bottom-1 left-1">
                <svg class="w-4 h-4 text-white drop-shadow" fill="currentColor" viewBox="0 0 20 20">
                  <path d="M6.3 2.841A1.5 1.5 0 004 4.11V15.89a1.5 1.5 0 002.3 1.269l9.344-5.89a1.5 1.5 0 000-2.538L6.3 2.84z" />
                </svg>
              </div>
            `
          : ''}
      </div>
    `;
  }
}
