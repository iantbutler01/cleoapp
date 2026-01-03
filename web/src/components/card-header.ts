import { LitElement, html } from 'lit';
import { customElement } from 'lit/decorators.js';
import { tailwindStyles } from '../styles/shared';

@customElement('card-header')
export class CardHeader extends LitElement {
  static styles = [tailwindStyles];

  render() {
    return html`
      <div class="flex justify-between items-center mb-3 text-base-content/70">
        <slot name="left"></slot>
        <div class="flex items-center gap-2">
          <slot name="right"></slot>
        </div>
      </div>
    `;
  }
}
