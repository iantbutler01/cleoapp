import { LitElement, html, css } from 'lit';
import { customElement, property, state, query } from 'lit/decorators.js';
import { tailwindStyles } from '../styles/shared';

/** Aspect ratio presets */
const ASPECT_RATIOS = [
  { label: 'Free', value: null },
  { label: '1:1', value: 1 },
  { label: '16:9', value: 16 / 9 },
  { label: '4:3', value: 4 / 3 },
  { label: '9:16', value: 9 / 16 },
] as const;

@customElement('image-cropper')
export class ImageCropper extends LitElement {
  static styles = [
    tailwindStyles,
    css`
      :host {
        display: flex;
        flex-direction: column;
        height: 100%;
      }
      .cropper-container {
        flex: 1;
        min-height: 0;
        position: relative;
        display: flex;
        align-items: center;
        justify-content: center;
        background: oklch(var(--b2));
        border-radius: 8px;
        overflow: hidden;
      }
      .image-wrapper {
        position: relative;
        max-width: 100%;
        max-height: 100%;
      }
      .source-image {
        max-width: 100%;
        max-height: 100%;
        display: block;
      }
      .crop-overlay {
        position: absolute;
        top: 0;
        left: 0;
        right: 0;
        bottom: 0;
        pointer-events: none;
      }
      .crop-mask {
        position: absolute;
        top: 0;
        left: 0;
        right: 0;
        bottom: 0;
        background: rgba(0, 0, 0, 0.5);
        clip-path: polygon(
          0% 0%, 0% 100%, var(--crop-left) 100%, var(--crop-left) var(--crop-top),
          var(--crop-right) var(--crop-top), var(--crop-right) var(--crop-bottom),
          var(--crop-left) var(--crop-bottom), var(--crop-left) 100%, 100% 100%, 100% 0%
        );
      }
      .crop-box {
        position: absolute;
        border: 2px solid oklch(var(--p));
        box-shadow: 0 0 0 9999px rgba(0, 0, 0, 0.5);
        cursor: move;
        pointer-events: auto;
      }
      .crop-handle {
        position: absolute;
        width: 12px;
        height: 12px;
        background: oklch(var(--p));
        border: 2px solid white;
        border-radius: 2px;
        pointer-events: auto;
      }
      .crop-handle.nw { top: -6px; left: -6px; cursor: nwse-resize; }
      .crop-handle.ne { top: -6px; right: -6px; cursor: nesw-resize; }
      .crop-handle.sw { bottom: -6px; left: -6px; cursor: nesw-resize; }
      .crop-handle.se { bottom: -6px; right: -6px; cursor: nwse-resize; }
      .crop-handle.n { top: -6px; left: 50%; transform: translateX(-50%); cursor: ns-resize; }
      .crop-handle.s { bottom: -6px; left: 50%; transform: translateX(-50%); cursor: ns-resize; }
      .crop-handle.w { left: -6px; top: 50%; transform: translateY(-50%); cursor: ew-resize; }
      .crop-handle.e { right: -6px; top: 50%; transform: translateY(-50%); cursor: ew-resize; }
    `,
  ];

  @property({ type: String }) imageUrl: string | null = null;
  @property({ type: Number }) cropX = 0;
  @property({ type: Number }) cropY = 0;
  @property({ type: Number }) cropWidth = 1;
  @property({ type: Number }) cropHeight = 1;

  @state() private imageLoaded = false;
  @state() private imageWidth = 0;
  @state() private imageHeight = 0;
  @state() private aspectRatio: number | null = null;
  @state() private dragging: 'move' | 'nw' | 'ne' | 'sw' | 'se' | 'n' | 's' | 'e' | 'w' | null = null;
  @state() private dragStartX = 0;
  @state() private dragStartY = 0;
  @state() private dragStartCrop = { x: 0, y: 0, width: 0, height: 0 };

  @query('.source-image') private imageEl!: HTMLImageElement;
  @query('.image-wrapper') private wrapperEl!: HTMLDivElement;

  connectedCallback() {
    super.connectedCallback();
    window.addEventListener('mousemove', this.handleMouseMove);
    window.addEventListener('mouseup', this.handleMouseUp);
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    window.removeEventListener('mousemove', this.handleMouseMove);
    window.removeEventListener('mouseup', this.handleMouseUp);
  }

  private handleImageLoad() {
    this.imageLoaded = true;
    this.imageWidth = this.imageEl.naturalWidth;
    this.imageHeight = this.imageEl.naturalHeight;
  }

  private setAspectRatio(ratio: number | null) {
    this.aspectRatio = ratio;
    if (ratio) {
      // Calculate target aspect ratio in image-normalized coordinates
      const targetAspect = ratio * (this.imageHeight / this.imageWidth);
      const currentAspect = this.cropWidth / this.cropHeight;

      let newWidth = this.cropWidth;
      let newHeight = this.cropHeight;
      let newX = this.cropX;
      let newY = this.cropY;

      if (currentAspect > targetAspect) {
        // Too wide, reduce width to match aspect
        newWidth = this.cropHeight * targetAspect;
      } else {
        // Too tall, reduce height to match aspect
        newHeight = this.cropWidth / targetAspect;
      }

      // Ensure dimensions don't exceed bounds (max 1.0)
      if (newWidth > 1) {
        newWidth = 1;
        newHeight = newWidth / targetAspect;
      }
      if (newHeight > 1) {
        newHeight = 1;
        newWidth = newHeight * targetAspect;
      }

      // Center the crop box, clamping to valid bounds
      const diffX = this.cropWidth - newWidth;
      const diffY = this.cropHeight - newHeight;
      newX = Math.max(0, Math.min(1 - newWidth, this.cropX + diffX / 2));
      newY = Math.max(0, Math.min(1 - newHeight, this.cropY + diffY / 2));

      this.cropX = newX;
      this.cropY = newY;
      this.cropWidth = newWidth;
      this.cropHeight = newHeight;
      this.emitCropChange();
    }
  }

  private handleMouseDown = (e: MouseEvent, type: typeof this.dragging) => {
    e.preventDefault();
    e.stopPropagation();
    this.dragging = type;
    this.dragStartX = e.clientX;
    this.dragStartY = e.clientY;
    this.dragStartCrop = {
      x: this.cropX,
      y: this.cropY,
      width: this.cropWidth,
      height: this.cropHeight,
    };
  };

  private handleMouseMove = (e: MouseEvent) => {
    if (!this.dragging || !this.wrapperEl) return;

    const rect = this.wrapperEl.getBoundingClientRect();
    const deltaX = (e.clientX - this.dragStartX) / rect.width;
    const deltaY = (e.clientY - this.dragStartY) / rect.height;

    let newX = this.dragStartCrop.x;
    let newY = this.dragStartCrop.y;
    let newWidth = this.dragStartCrop.width;
    let newHeight = this.dragStartCrop.height;

    if (this.dragging === 'move') {
      newX = Math.max(0, Math.min(1 - newWidth, this.dragStartCrop.x + deltaX));
      newY = Math.max(0, Math.min(1 - newHeight, this.dragStartCrop.y + deltaY));
    } else {
      // Handle resize based on which handle is being dragged
      const minSize = 0.05; // Minimum 5% of image

      if (this.dragging.includes('w')) {
        const maxDelta = this.dragStartCrop.width - minSize;
        const clampedDelta = Math.max(-this.dragStartCrop.x, Math.min(maxDelta, deltaX));
        newX = this.dragStartCrop.x + clampedDelta;
        newWidth = this.dragStartCrop.width - clampedDelta;
      }
      if (this.dragging.includes('e')) {
        newWidth = Math.max(minSize, Math.min(1 - newX, this.dragStartCrop.width + deltaX));
      }
      if (this.dragging.includes('n')) {
        const maxDelta = this.dragStartCrop.height - minSize;
        const clampedDelta = Math.max(-this.dragStartCrop.y, Math.min(maxDelta, deltaY));
        newY = this.dragStartCrop.y + clampedDelta;
        newHeight = this.dragStartCrop.height - clampedDelta;
      }
      if (this.dragging.includes('s')) {
        newHeight = Math.max(minSize, Math.min(1 - newY, this.dragStartCrop.height + deltaY));
      }

      // Apply aspect ratio constraint
      if (this.aspectRatio) {
        const targetAspect = this.aspectRatio * (this.imageHeight / this.imageWidth);

        if (this.dragging === 'n' || this.dragging === 's') {
          // Adjust width to match aspect ratio
          newWidth = newHeight * targetAspect;
        } else {
          // Adjust height to match aspect ratio
          newHeight = newWidth / targetAspect;
        }

        // Clamp to image bounds
        if (newX + newWidth > 1) {
          newWidth = 1 - newX;
          newHeight = newWidth / targetAspect;
        }
        if (newY + newHeight > 1) {
          newHeight = 1 - newY;
          newWidth = newHeight * targetAspect;
        }
        if (newWidth > 1) {
          newWidth = 1;
          newHeight = newWidth / targetAspect;
        }
        if (newHeight > 1) {
          newHeight = 1;
          newWidth = newHeight * targetAspect;
        }
      }
    }

    // Final bounds clamp to ensure 0-1 range
    this.cropX = Math.max(0, Math.min(1 - newWidth, newX));
    this.cropY = Math.max(0, Math.min(1 - newHeight, newY));
    this.cropWidth = Math.max(0.05, Math.min(1, newWidth));
    this.cropHeight = Math.max(0.05, Math.min(1, newHeight));
    this.emitCropChange();
  };

  private handleMouseUp = () => {
    this.dragging = null;
  };

  private emitCropChange() {
    this.dispatchEvent(
      new CustomEvent('crop-change', {
        detail: {
          x: this.cropX,
          y: this.cropY,
          width: this.cropWidth,
          height: this.cropHeight,
        },
        bubbles: true,
        composed: true,
      })
    );
  }

  render() {
    const cropBoxStyle = `
      left: ${this.cropX * 100}%;
      top: ${this.cropY * 100}%;
      width: ${this.cropWidth * 100}%;
      height: ${this.cropHeight * 100}%;
    `;

    return html`
      <!-- Aspect Ratio Presets -->
      <div class="flex gap-2 mb-3">
        ${ASPECT_RATIOS.map(
          (ratio) => html`
            <button
              class="btn btn-sm ${this.aspectRatio === ratio.value ? 'btn-primary' : 'btn-ghost'}"
              @click=${() => this.setAspectRatio(ratio.value)}
            >
              ${ratio.label}
            </button>
          `
        )}
      </div>

      <!-- Cropper -->
      <div class="cropper-container">
        ${this.imageUrl
          ? html`
              <div class="image-wrapper">
                <img
                  class="source-image"
                  src=${this.imageUrl}
                  @load=${this.handleImageLoad}
                  draggable="false"
                />
                ${this.imageLoaded
                  ? html`
                      <div class="crop-overlay">
                        <div
                          class="crop-box"
                          style=${cropBoxStyle}
                          @mousedown=${(e: MouseEvent) => this.handleMouseDown(e, 'move')}
                        >
                          <!-- Corner handles -->
                          <div class="crop-handle nw" @mousedown=${(e: MouseEvent) => this.handleMouseDown(e, 'nw')}></div>
                          <div class="crop-handle ne" @mousedown=${(e: MouseEvent) => this.handleMouseDown(e, 'ne')}></div>
                          <div class="crop-handle sw" @mousedown=${(e: MouseEvent) => this.handleMouseDown(e, 'sw')}></div>
                          <div class="crop-handle se" @mousedown=${(e: MouseEvent) => this.handleMouseDown(e, 'se')}></div>
                          <!-- Edge handles -->
                          <div class="crop-handle n" @mousedown=${(e: MouseEvent) => this.handleMouseDown(e, 'n')}></div>
                          <div class="crop-handle s" @mousedown=${(e: MouseEvent) => this.handleMouseDown(e, 's')}></div>
                          <div class="crop-handle w" @mousedown=${(e: MouseEvent) => this.handleMouseDown(e, 'w')}></div>
                          <div class="crop-handle e" @mousedown=${(e: MouseEvent) => this.handleMouseDown(e, 'e')}></div>
                        </div>
                      </div>
                    `
                  : ''}
              </div>
            `
          : html`
              <div class="flex items-center justify-center text-base-content/50">
                No image loaded
              </div>
            `}
      </div>

      <!-- Crop Info -->
      <div class="mt-3 text-sm text-base-content/60 flex gap-4">
        <span>X: ${(this.cropX * 100).toFixed(1)}%</span>
        <span>Y: ${(this.cropY * 100).toFixed(1)}%</span>
        <span>Width: ${(this.cropWidth * 100).toFixed(1)}%</span>
        <span>Height: ${(this.cropHeight * 100).toFixed(1)}%</span>
      </div>
    `;
  }
}
