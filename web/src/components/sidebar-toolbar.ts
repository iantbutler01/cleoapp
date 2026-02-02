import { LitElement, html } from "lit";
import { customElement } from "lit/decorators.js";
import { tailwindStyles } from "../styles/shared";

@customElement("sidebar-toolbar")
export class SidebarToolbar extends LitElement {
  static styles = [tailwindStyles];

  private openNudges() {
    this.dispatchEvent(
      new CustomEvent("open-nudges", { bubbles: true, composed: true })
    );
  }

  render() {
    return html`
      <div
        class="sticky top-24 mt-2 flex flex-col gap-1 bg-base-100/90 backdrop-blur-sm rounded-r-lg py-1.5 px-2 border border-l-0 border-base-300/30 shadow-sm -ml-px"
      >
        <button
          class="w-6 h-6 flex items-center justify-center rounded text-base-content/50 hover:text-base-content hover:bg-base-200/50 transition-colors tooltip tooltip-right"
          data-tip="Voice & Style (⌘+.)"
          @click=${this.openNudges}
        >
          <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
              d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z" />
          </svg>
        </button>
      </div>
    `;
  }
}
