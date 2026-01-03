import { LitElement, html, css } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import { tailwindStyles } from '../styles/shared';

@customElement('content-badge')
export class ContentBadge extends LitElement {
  static styles = [
    tailwindStyles,
    css`
      :host {
        display: inline-flex;
      }
    `
  ];

  @property({ type: String }) variant: 'muted' | 'accent' | 'status-success' | 'status-warning' = 'muted';

  render() {
    const base = 'text-xs px-2 py-0.5 rounded-full font-medium';

    const variants: Record<string, string> = {
      muted: 'bg-base-200 text-base-content/70',
      accent: 'bg-primary/20 text-primary',
      'status-success': 'bg-success/20 text-success',
      'status-warning': 'bg-warning/20 text-warning',
    };

    return html`
      <span class="${base} ${variants[this.variant] || variants.muted}">
        <slot></slot>
      </span>
    `;
  }
}
