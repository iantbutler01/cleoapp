import { LitElement, html, css } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { api, apiBaseForWs } from '../api';
import { tailwindStyles } from '../styles/shared';
import './image-cropper';
import './video-trimmer';

/** WebSocket command to send */
interface EditCommand {
  action: 'crop' | 'trim';
  capture_id: number;
  // Crop params
  x?: number;
  y?: number;
  width?: number;
  height?: number;
  // Trim params
  start?: string;
  duration?: number;
}

/** WebSocket response */
interface EditResponse {
  type: 'progress' | 'complete' | 'error';
  percent?: number;
  status?: string;
  new_capture_id?: number;
  message?: string;
}

@customElement('media-editor')
export class MediaEditor extends LitElement {
  static styles = [
    tailwindStyles,
    css`
      .editor-content {
        display: flex;
        flex-direction: column;
        height: 100%;
        min-height: 0;
      }
      .preview-container {
        flex: 1;
        min-height: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        background: oklch(var(--b2));
        border-radius: 8px;
        overflow: hidden;
      }
    `,
  ];

  @property({ type: Boolean }) open = false;
  @property({ type: Number }) captureId: number | null = null;
  @property({ type: String }) mediaType: 'image' | 'video' = 'image';

  @state() private previewUrl: string | null = null;
  @state() private loading = true;
  @state() private loadError: string | null = null;
  @state() private processing = false;
  @state() private progress = 0;
  @state() private progressStatus = '';
  @state() private error: string | null = null;

  // Video info
  @state() private videoDuration = 0;

  // Crop state (normalized 0-1)
  @state() private cropX = 0;
  @state() private cropY = 0;
  @state() private cropWidth = 1;
  @state() private cropHeight = 1;

  // Trim state
  @state() private trimStart = '00:00:00';
  @state() private trimDuration = 30;

  private ws: WebSocket | null = null;

  async connectedCallback() {
    super.connectedCallback();
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    this.closeWs();
  }

  protected updated(changedProperties: Map<string, unknown>) {
    if (changedProperties.has('open') && this.open && this.captureId) {
      this.loadPreview();
      this.connectWs();
    }
    if (changedProperties.has('open') && !this.open) {
      this.closeWs();
    }
  }

  private async loadPreview() {
    if (!this.captureId) return;

    this.loading = true;
    this.loadError = null;
    this.previewUrl = null;

    try {
      const result = await api.getCaptureUrl(this.captureId);
      this.previewUrl = result.url;
    } catch (e) {
      console.error('Failed to load preview:', e);
      this.loadError = e instanceof Error ? e.message : 'Failed to load media';
    } finally {
      this.loading = false;
    }
  }

  private connectWs() {
    if (this.ws) return;

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const wsUrl = `${protocol}//${apiBaseForWs.origin}${apiBaseForWs.path}/media/edit/ws`;

    this.ws = new WebSocket(wsUrl);

    this.ws.onopen = () => {
      console.log('[media-editor] WebSocket connected');
    };

    this.ws.onmessage = (event) => {
      try {
        const response: EditResponse = JSON.parse(event.data);
        this.handleWsMessage(response);
      } catch (e) {
        console.error('[media-editor] Failed to parse WebSocket message:', e);
      }
    };

    this.ws.onerror = (e) => {
      console.error('[media-editor] WebSocket error:', e);
      this.error = 'Connection error';
    };

    this.ws.onclose = () => {
      console.log('[media-editor] WebSocket closed');
      this.ws = null;
    };
  }

  private closeWs() {
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
  }

  private handleWsMessage(response: EditResponse) {
    switch (response.type) {
      case 'progress':
        this.progress = response.percent ?? 0;
        this.progressStatus = response.status ?? '';
        break;

      case 'complete':
        this.processing = false;
        this.progress = 100;
        console.log('[media-editor] Edit complete - new_capture_id from server:', response.new_capture_id);
        // Dispatch event with new capture ID
        this.dispatchEvent(
          new CustomEvent('edit-complete', {
            detail: { newCaptureId: response.new_capture_id },
            bubbles: true,
            composed: true,
          })
        );
        this.close();
        break;

      case 'error':
        this.processing = false;
        this.error = response.message ?? 'Edit failed';
        break;
    }
  }

  private handleApply() {
    if (!this.ws || !this.captureId) return;

    this.processing = true;
    this.error = null;
    this.progress = 0;

    let cmd: EditCommand;

    if (this.mediaType === 'image') {
      cmd = {
        action: 'crop',
        capture_id: this.captureId,
        x: this.cropX,
        y: this.cropY,
        width: this.cropWidth,
        height: this.cropHeight,
      };
    } else {
      cmd = {
        action: 'trim',
        capture_id: this.captureId,
        start: this.trimStart,
        duration: this.trimDuration,
      };
    }

    this.ws.send(JSON.stringify(cmd));
  }

  private handleCropChange(e: CustomEvent<{ x: number; y: number; width: number; height: number }>) {
    const { x, y, width, height } = e.detail;
    this.cropX = x;
    this.cropY = y;
    this.cropWidth = width;
    this.cropHeight = height;
  }

  private handleTrimChange(e: CustomEvent<{ start: string; duration: number }>) {
    const { start, duration } = e.detail;
    this.trimStart = start;
    this.trimDuration = duration;
  }

  private handleVideoMetadata(e: CustomEvent<{ duration: number }>) {
    this.videoDuration = e.detail.duration;
    // Default trim to full video or max 60 seconds
    this.trimDuration = Math.min(this.videoDuration, 60);
  }

  private close() {
    this.open = false;
    this.previewUrl = null;
    this.error = null;
    this.processing = false;
    this.dispatchEvent(new CustomEvent('close', { bubbles: true, composed: true }));
  }

  render() {
    if (!this.open) return html``;

    return html`
      <div class="modal modal-open">
        <div class="modal-box ${this.mediaType === 'video' ? 'max-w-6xl' : 'max-w-4xl'} h-[90vh] flex flex-col">
          <!-- Header -->
          <div class="flex justify-between items-center mb-4">
            <h3 class="font-bold text-lg">
              ${this.mediaType === 'image' ? 'Crop Image' : 'Trim Video'}
            </h3>
            <button
              class="btn btn-sm btn-circle btn-ghost"
              @click=${this.close}
              ?disabled=${this.processing}
            >
              ✕
            </button>
          </div>

          <!-- Content -->
          <div class="editor-content flex-1 min-h-0">
            ${this.loading
              ? html`
                  <div class="flex-1 flex items-center justify-center">
                    <span class="loading loading-spinner loading-lg"></span>
                  </div>
                `
              : this.loadError
                ? html`
                    <div class="flex-1 flex items-center justify-center">
                      <div class="text-center">
                        <p class="text-error mb-2">${this.loadError}</p>
                        <button class="btn btn-sm" @click=${this.loadPreview}>Retry</button>
                      </div>
                    </div>
                  `
                : this.mediaType === 'image'
                  ? html`
                      <image-cropper
                        .imageUrl=${this.previewUrl}
                        .cropX=${this.cropX}
                        .cropY=${this.cropY}
                        .cropWidth=${this.cropWidth}
                        .cropHeight=${this.cropHeight}
                        @crop-change=${this.handleCropChange}
                      ></image-cropper>
                    `
                  : html`
                      <video-trimmer
                        .videoUrl=${this.previewUrl}
                        .startTimestamp=${this.trimStart}
                        .durationSecs=${this.trimDuration}
                        @trim-change=${this.handleTrimChange}
                        @video-metadata=${this.handleVideoMetadata}
                      ></video-trimmer>
                    `}
          </div>

          <!-- Error -->
          ${this.error
            ? html`
                <div class="alert alert-error mt-3 py-2">
                  <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/>
                  </svg>
                  <span class="text-sm">${this.error}</span>
                </div>
              `
            : ''}

          <!-- Progress -->
          ${this.processing
            ? html`
                <div class="mt-3">
                  <div class="flex justify-between text-sm mb-1">
                    <span>${this.progressStatus || 'Processing...'}</span>
                    <span>${this.progress}%</span>
                  </div>
                  <progress class="progress progress-primary w-full" value=${this.progress} max="100"></progress>
                </div>
              `
            : ''}

          <!-- Actions -->
          <div class="modal-action mt-4">
            <button class="btn btn-ghost" @click=${this.close} ?disabled=${this.processing}>
              Cancel
            </button>
            <button
              class="btn btn-primary"
              @click=${this.handleApply}
              ?disabled=${this.processing || this.loading || !!this.loadError}
            >
              ${this.processing
                ? html`<span class="loading loading-spinner loading-sm"></span>`
                : this.mediaType === 'image'
                  ? 'Apply Crop'
                  : 'Apply Trim'}
            </button>
          </div>
        </div>
        <div class="modal-backdrop bg-black/50" @click=${() => !this.processing && this.close()}></div>
      </div>
    `;
  }
}
