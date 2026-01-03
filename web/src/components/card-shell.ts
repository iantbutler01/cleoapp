import { LitElement, html } from 'lit';
import { customElement, property } from 'lit/decorators.js';
import { tailwindStyles } from '../styles/shared';

@customElement('card-shell')
export class CardShell extends LitElement {
  static styles = [tailwindStyles];

  @property({ type: Boolean }) showRailConnector = false;
  @property({ type: String }) variant: 'default' | 'compact' = 'default';

  render() {
    const padding = this.variant === 'compact' ? 'p-3' : 'p-5';

    return html`
      <div class="bg-base-100 rounded-2xl border border-base-300/50 ${padding} relative shadow-sm hover:shadow-md transition-shadow max-w-xl">
        ${this.showRailConnector ? html`
          <div class="absolute -left-4 top-6 w-4 h-0.5 bg-base-300/50"></div>
        ` : ''}

        <slot name="header"></slot>
        <slot></slot>
        <slot name="actions"></slot>
      </div>
    `;
  }
}
