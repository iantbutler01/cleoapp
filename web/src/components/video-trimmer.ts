import { LitElement, html, css } from 'lit';
import { customElement, property, state, query } from 'lit/decorators.js';
import { tailwindStyles } from '../styles/shared';

/** Format seconds to timestamp string */
function formatTimestamp(secs: number): string {
  const hours = Math.floor(secs / 3600);
  const mins = Math.floor((secs % 3600) / 60);
  const s = Math.floor(secs % 60);
  if (hours > 0) {
    return `${hours.toString().padStart(2, '0')}:${mins.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`;
  }
  return `${mins.toString().padStart(2, '0')}:${s.toString().padStart(2, '0')}`;
}

/** Parse timestamp string to seconds */
function parseTimestamp(ts: string): number {
  const parts = ts.split(':').map(Number);
  if (parts.length === 3) {
    return parts[0] * 3600 + parts[1] * 60 + parts[2];
  } else if (parts.length === 2) {
    return parts[0] * 60 + parts[1];
  }
  return Number(ts) || 0;
}

@customElement('video-trimmer')
export class VideoTrimmer extends LitElement {
  static styles = [
    tailwindStyles,
    css`
      :host {
        display: flex;
        flex-direction: column;
        height: 100%;
      }
      .video-container {
        flex: 1;
        min-height: 0;
        display: flex;
        align-items: center;
        justify-content: center;
        background: black;
        border-radius: 8px;
        overflow: hidden;
      }
      video {
        width: 100%;
        height: 100%;
        object-fit: contain;
      }
      .timeline-container {
        margin-top: 1rem;
        padding: 0 8px;
      }
      .timeline-track {
        position: relative;
        height: 56px;
        background: oklch(var(--b3));
        border-radius: 8px;
        cursor: pointer;
      }
      /* Thumbnail strip container */
      .timeline-thumbnails {
        position: absolute;
        top: 0;
        left: 0;
        right: 0;
        bottom: 0;
        display: flex;
        overflow: hidden;
        border-radius: 8px;
      }
      .timeline-thumbnails canvas {
        height: 100%;
        flex-shrink: 0;
      }
      /* Selected region highlight overlay */
      .timeline-selection {
        position: absolute;
        top: 0;
        bottom: 0;
        background: var(--color-primary);
        opacity: 0.3;
        pointer-events: none;
      }
      /* Handle brackets - primary colored chunky handles */
      .timeline-handle {
        position: absolute;
        top: 0;
        bottom: 0;
        width: 14px;
        cursor: ew-resize;
        background: var(--color-primary);
        z-index: 10;
      }
      .timeline-handle.start {
        border-radius: 4px 0 0 4px;
      }
      .timeline-handle.end {
        border-radius: 0 4px 4px 0;
      }
      .timeline-handle:hover {
        background: var(--color-secondary);
      }
      /* Playhead */
      .timeline-playhead {
        position: absolute;
        top: 0;
        bottom: 0;
        width: 3px;
        background: white;
        pointer-events: none;
        z-index: 15;
        box-shadow: 0 0 6px rgba(0,0,0,0.5);
        border-radius: 1px;
      }
      /* Time display below timeline */
      .time-display {
        display: flex;
        justify-content: space-between;
        align-items: center;
        margin-top: 8px;
        font-size: 12px;
        font-family: monospace;
        color: oklch(var(--bc) / 0.6);
      }
      .time-display .selection-range {
        color: var(--color-primary);
        font-weight: 500;
      }
      .time-inputs {
        display: flex;
        gap: 1rem;
        margin-top: 0.75rem;
      }
      .time-input-group {
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
      }
      .time-input-group label {
        font-size: 0.75rem;
        opacity: 0.6;
      }
      .time-input-group input {
        width: 80px;
      }
    `,
  ];

  @property({ type: String }) videoUrl: string | null = null;
  @property({ type: String }) startTimestamp = '00:00:00';
  @property({ type: Number }) durationSecs = 30;

  // Internal state - working values (can be modified freely)
  @state() private _startSecs = 0;
  @state() private _endSecs = 30;
  @state() private videoDuration = 0;
  @state() private currentTime = 0;
  @state() private playing = false;
  @state() private dragging: 'start' | 'end' | null = null;
  private wasDragging = false;

  // Undo/redo history
  private undoStack: Array<{ start: number; end: number }> = [];
  private redoStack: Array<{ start: number; end: number }> = [];
  @state() private canUndo = false;
  @state() private canRedo = false;

  @query('video') private videoEl!: HTMLVideoElement;
  @query('.timeline-track') private trackEl!: HTMLDivElement;
  @query('.timeline-canvas') private canvasEl!: HTMLCanvasElement;

  connectedCallback() {
    super.connectedCallback();
    // Initialize from props
    this._startSecs = parseTimestamp(this.startTimestamp);
    this._endSecs = this._startSecs + this.durationSecs;
    window.addEventListener('mousemove', this.handleMouseMove);
    window.addEventListener('mouseup', this.handleMouseUp);
    window.addEventListener('keydown', this.handleKeyDown);
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    window.removeEventListener('mousemove', this.handleMouseMove);
    window.removeEventListener('mouseup', this.handleMouseUp);
    window.removeEventListener('keydown', this.handleKeyDown);
  }

  private handleKeyDown = (e: KeyboardEvent) => {
    // Cmd/Ctrl+Z for undo, Cmd/Ctrl+Shift+Z for redo
    if ((e.metaKey || e.ctrlKey) && e.key === 'z') {
      e.preventDefault();
      if (e.shiftKey) {
        this.redo();
      } else {
        this.undo();
      }
    }
    // Cmd/Ctrl+Y for redo (Windows style)
    if ((e.metaKey || e.ctrlKey) && e.key === 'y') {
      e.preventDefault();
      this.redo();
    }
  };

  private handleVideoMetadata() {
    this.videoDuration = this.videoEl.duration;
    this.dispatchEvent(
      new CustomEvent('video-metadata', {
        detail: { duration: this.videoDuration },
        bubbles: true,
        composed: true,
      })
    );
    // Initialize selection if needed
    if (this._endSecs <= 0 || this._endSecs > this.videoDuration) {
      this._endSecs = Math.min(this.videoDuration, this._startSecs + 30);
      this.emitTrimChange();
    }
    // Generate thumbnails after a brief delay to ensure canvas is ready
    setTimeout(() => this.generateThumbnails(), 100);
  }

  private async generateThumbnails() {
    if (!this.videoEl || !this.canvasEl || !this.trackEl) return;

    const trackWidth = this.trackEl.clientWidth;
    const trackHeight = this.trackEl.clientHeight;
    const thumbWidth = Math.ceil((trackHeight * this.videoEl.videoWidth) / this.videoEl.videoHeight);
    const numThumbs = Math.ceil(trackWidth / thumbWidth) + 1;

    // Size canvas to fit all thumbnails
    this.canvasEl.width = thumbWidth * numThumbs;
    this.canvasEl.height = trackHeight;
    this.canvasEl.style.width = `${thumbWidth * numThumbs}px`;

    const ctx = this.canvasEl.getContext('2d');
    if (!ctx) return;

    // Create offscreen video for seeking
    const offscreenVideo = document.createElement('video');
    offscreenVideo.src = this.videoEl.src;
    offscreenVideo.muted = true;
    offscreenVideo.preload = 'metadata';

    await new Promise<void>((resolve) => {
      offscreenVideo.onloadedmetadata = () => resolve();
    });

    for (let i = 0; i < numThumbs; i++) {
      const time = (i / numThumbs) * this.videoDuration;
      offscreenVideo.currentTime = time;

      await new Promise<void>((resolve) => {
        offscreenVideo.onseeked = () => {
          ctx.drawImage(offscreenVideo, i * thumbWidth, 0, thumbWidth, trackHeight);
          resolve();
        };
      });
    }
  }

  private handleTimeUpdate() {
    this.currentTime = this.videoEl.currentTime;
    if (this.playing && this.currentTime >= this._endSecs) {
      this.videoEl.currentTime = this._startSecs;
    }
  }

  private togglePlay() {
    if (this.playing) {
      this.videoEl.pause();
    } else {
      if (this.currentTime < this._startSecs || this.currentTime >= this._endSecs) {
        this.videoEl.currentTime = this._startSecs;
      }
      this.videoEl.play();
    }
    this.playing = !this.playing;
  }

  private handleTrackClick(e: MouseEvent) {
    // Don't move playhead if we just finished dragging a handle
    if (this.wasDragging) {
      this.wasDragging = false;
      return;
    }
    const rect = this.trackEl.getBoundingClientRect();
    const x = (e.clientX - rect.left) / rect.width;
    const time = x * this.videoDuration;
    this.videoEl.currentTime = time;
  }

  private handleDragStart = (e: MouseEvent, type: 'start' | 'end') => {
    e.preventDefault();
    e.stopPropagation();
    // Save state before drag begins
    this.pushUndoState();
    this.dragging = type;
  };

  private handleMouseMove = (e: MouseEvent) => {
    if (!this.dragging || !this.trackEl) return;
    e.preventDefault(); // Prevent text selection while dragging

    const rect = this.trackEl.getBoundingClientRect();
    const x = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
    const time = x * this.videoDuration;

    if (this.dragging === 'start') {
      const maxStart = this._endSecs - 1;
      this._startSecs = Math.max(0, Math.min(maxStart, time));
    } else if (this.dragging === 'end') {
      const minEnd = this._startSecs + 1;
      this._endSecs = Math.max(minEnd, Math.min(this.videoDuration, time));
    }
  };

  private handleMouseUp = () => {
    if (this.dragging) {
      this.wasDragging = true;
      this.emitTrimChange();
      this.dragging = null;
    }
  };

  private handleStartInput(e: Event) {
    const input = e.target as HTMLInputElement;
    const newStart = parseTimestamp(input.value);
    if (newStart >= 0 && newStart < this._endSecs - 1) {
      this.pushUndoState();
      this._startSecs = newStart;
      this.emitTrimChange();
    }
  }

  private handleEndInput(e: Event) {
    const input = e.target as HTMLInputElement;
    const newEnd = parseTimestamp(input.value);
    if (newEnd > this._startSecs + 1 && newEnd <= this.videoDuration) {
      this.pushUndoState();
      this._endSecs = newEnd;
      this.emitTrimChange();
    }
  }

  private emitTrimChange() {
    this.dispatchEvent(
      new CustomEvent('trim-change', {
        detail: {
          start: formatTimestamp(this._startSecs),
          duration: this._endSecs - this._startSecs,
        },
        bubbles: true,
        composed: true,
      })
    );
  }

  private previewSelection() {
    this.videoEl.currentTime = this._startSecs;
    this.videoEl.play();
    this.playing = true;
  }

  /** Push current state to undo stack before making a change */
  private pushUndoState() {
    this.undoStack.push({ start: this._startSecs, end: this._endSecs });
    this.redoStack = []; // Clear redo stack on new action
    this.canUndo = true;
    this.canRedo = false;
  }

  /** Undo last change */
  private undo() {
    if (this.undoStack.length === 0) return;
    // Save current state to redo stack
    this.redoStack.push({ start: this._startSecs, end: this._endSecs });
    // Restore previous state
    const prev = this.undoStack.pop()!;
    this._startSecs = prev.start;
    this._endSecs = prev.end;
    this.canUndo = this.undoStack.length > 0;
    this.canRedo = true;
    this.emitTrimChange();
  }

  /** Redo last undone change */
  private redo() {
    if (this.redoStack.length === 0) return;
    // Save current state to undo stack
    this.undoStack.push({ start: this._startSecs, end: this._endSecs });
    // Restore next state
    const next = this.redoStack.pop()!;
    this._startSecs = next.start;
    this._endSecs = next.end;
    this.canUndo = true;
    this.canRedo = this.redoStack.length > 0;
    this.emitTrimChange();
  }

  render() {
    const startPercent = this.videoDuration > 0 ? (this._startSecs / this.videoDuration) * 100 : 0;
    const endPercent = this.videoDuration > 0 ? (this._endSecs / this.videoDuration) * 100 : 100;
    const playheadPercent = this.videoDuration > 0 ? (this.currentTime / this.videoDuration) * 100 : 0;
    const duration = this._endSecs - this._startSecs;

    return html`
      <!-- Video Preview -->
      <div class="video-container">
        ${this.videoUrl
          ? html`
              <video
                src=${this.videoUrl}
                @loadedmetadata=${this.handleVideoMetadata}
                @timeupdate=${this.handleTimeUpdate}
                @pause=${() => (this.playing = false)}
                @play=${() => (this.playing = true)}
              ></video>
            `
          : html`
              <div class="flex items-center justify-center text-base-content/50">
                No video loaded
              </div>
            `}
      </div>

      <!-- Controls -->
      <div class="flex items-center gap-2 mt-3">
        <button class="btn btn-sm btn-circle" @click=${this.togglePlay}>
          ${this.playing
            ? html`<svg class="w-4 h-4" fill="currentColor" viewBox="0 0 24 24"><path d="M6 4h4v16H6V4zm8 0h4v16h-4V4z"/></svg>`
            : html`<svg class="w-4 h-4" fill="currentColor" viewBox="0 0 24 24"><path d="M8 5v14l11-7z"/></svg>`}
        </button>
        <span class="text-sm font-mono">
          ${formatTimestamp(this.currentTime)} / ${formatTimestamp(this.videoDuration)}
        </span>
        <div class="ml-auto flex items-center gap-1">
          <button
            class="btn btn-sm btn-ghost"
            @click=${this.undo}
            ?disabled=${!this.canUndo}
            title="Undo (Cmd+Z)"
          >
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 10h10a8 8 0 018 8v2M3 10l6 6m-6-6l6-6" />
            </svg>
          </button>
          <button
            class="btn btn-sm btn-ghost"
            @click=${this.redo}
            ?disabled=${!this.canRedo}
            title="Redo (Cmd+Shift+Z)"
          >
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 10h-10a8 8 0 00-8 8v2M21 10l-6 6m6-6l-6-6" />
            </svg>
          </button>
          <button class="btn btn-sm btn-ghost" @click=${this.previewSelection}>
            Preview
          </button>
        </div>
      </div>

      <!-- Timeline -->
      <div class="timeline-container">
        <div class="timeline-track" @click=${this.handleTrackClick}>
          <!-- Thumbnail strip -->
          <div class="timeline-thumbnails">
            <canvas class="timeline-canvas"></canvas>
          </div>
          <!-- Selection highlight overlay -->
          <div
            class="timeline-selection"
            style="left: ${startPercent}%; width: ${endPercent - startPercent}%;"
          ></div>
          <!-- Start handle - positioned independently -->
          <div
            class="timeline-handle start"
            style="left: ${startPercent}%;"
            @mousedown=${(e: MouseEvent) => this.handleDragStart(e, 'start')}
          ></div>
          <!-- End handle - positioned independently -->
          <div
            class="timeline-handle end"
            style="left: calc(${endPercent}% - 14px);"
            @mousedown=${(e: MouseEvent) => this.handleDragStart(e, 'end')}
          ></div>
          <!-- Playhead -->
          <div class="timeline-playhead" style="left: ${playheadPercent}%;"></div>
        </div>

        <!-- Time display -->
        <div class="time-display">
          <span>0:00</span>
          <span class="selection-range">${formatTimestamp(this._startSecs)} - ${formatTimestamp(this._endSecs)}</span>
          <span>${formatTimestamp(this.videoDuration)}</span>
        </div>
      </div>

      <!-- Time Inputs -->
      <div class="time-inputs">
        <div class="time-input-group">
          <label>Start</label>
          <input
            type="text"
            class="input input-bordered input-sm"
            .value=${formatTimestamp(this._startSecs)}
            @change=${this.handleStartInput}
          />
        </div>
        <div class="time-input-group">
          <label>Duration</label>
          <input
            type="text"
            class="input input-bordered input-sm"
            .value=${duration.toFixed(1) + 's'}
            readonly
          />
        </div>
        <div class="time-input-group">
          <label>End</label>
          <input
            type="text"
            class="input input-bordered input-sm"
            .value=${formatTimestamp(this._endSecs)}
            @change=${this.handleEndInput}
          />
        </div>
      </div>

      <!-- Selection Info -->
      <div class="mt-3 text-sm text-base-content/60">
        Selected: ${formatTimestamp(this._startSecs)} - ${formatTimestamp(this._endSecs)}
        (${duration.toFixed(1)}s)
      </div>
    `;
  }
}
