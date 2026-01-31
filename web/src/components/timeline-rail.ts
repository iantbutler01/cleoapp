import { LitElement, html } from "lit";
import { customElement, property } from "lit/decorators.js";
import { ContentItem } from "../api";
import { TimelineSection, formatTimeOfDay } from "../utils/time-grouping";
import { tailwindStyles } from "../styles/shared";

@customElement("timeline-rail")
export class TimelineRail extends LitElement {
  static styles = [tailwindStyles];

  @property({ type: Array }) sections: TimelineSection[] = [];
  @property({ type: Array }) content: ContentItem[] = [];
  @property({ attribute: false }) scrollTarget: HTMLElement | null = null;
  @property({ type: Number }) activeItemId: number | null = null;

  private handleRailWheel = (e: WheelEvent) => {
    if (this.scrollTarget) {
      e.preventDefault();
      this.scrollTarget.scrollBy({ top: e.deltaY });
    }
  };

  private handleDotClick(itemId: number) {
    this.dispatchEvent(
      new CustomEvent("dot-click", {
        detail: { itemId },
        bubbles: true,
      })
    );
  }

  render() {
    return html`
      <div
        class="sticky top-24 flex flex-col items-center bg-base-100/80 backdrop-blur-sm rounded-2xl py-4 px-3 border border-base-300/30 shadow-sm"
        @wheel=${this.handleRailWheel}
      >
        <div
          class="absolute left-1/2 top-4 bottom-4 w-px bg-base-300/50 -translate-x-1/2"
        ></div>
        ${this.sections.map(
          (section) => html`
            <div class="relative z-10 flex flex-col items-center">
              <div
                class="text-[9px] font-bold uppercase tracking-wider text-base-content/40 mb-2 px-1 py-0.5 bg-base-100 whitespace-nowrap"
              >
                ${section.label}
              </div>
              ${section.items.map((item) => {
                const itemId =
                  item.type === "tweet" ? item.id : item.thread.id;
                const isActive = this.activeItemId === itemId;
                const time = formatTimeOfDay(
                  item.type === "tweet" ? item.created_at : item.thread.created_at
                );
                return html`
                  <button
                    class="relative w-2 h-2 my-2 rounded-full transition-all group
                    ${isActive
                      ? "bg-primary scale-150 ring-4 ring-primary/20"
                      : "bg-base-300 hover:bg-primary/70 hover:scale-125"}"
                    @click=${() => this.handleDotClick(itemId)}
                  >
                    <div
                      class="absolute left-full ml-3 top-1/2 -translate-y-1/2 opacity-0 group-hover:opacity-100
                        transition-opacity pointer-events-none whitespace-nowrap bg-base-100 border border-base-300
                        px-2 py-1 rounded-lg shadow-lg text-xs z-20"
                    >
                      ${item.type === "thread" ? "Thread · " : ""}${time}
                    </div>
                  </button>
                `;
              })}
              <div class="h-3"></div>
            </div>
          `
        )}
      </div>
    `;
  }
}
