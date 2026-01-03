import { LitElement, html } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import { ContentItem } from "../api";
import { TimelineSection, formatTimeOfDay } from "../utils/time-grouping";
import { tailwindStyles } from "../styles/shared";

interface ItemPosition {
  id: number;
  top: number;
  height: number;
}

@customElement("timeline-rail")
export class TimelineRail extends LitElement {
  static styles = [tailwindStyles];

  @property({ type: Array }) sections: TimelineSection[] = [];
  @property({ type: Array }) content: ContentItem[] = [];

  @state() activeItemId: number | null = null;
  @state() itemPositions: ItemPosition[] = [];
  @state() indicatorTop = 0;

  private scrollContainer: HTMLElement | null = null;
  private rafId: number | null = null;

  connectedCallback() {
    super.connectedCallback();
    // Defer setup to allow DOM to render
    requestAnimationFrame(() => {
      this.setupScrollTracking();
      this.setupSlotListener();
    });
  }

  private setupSlotListener() {
    const slot = this.shadowRoot?.querySelector("slot");
    if (slot) {
      slot.addEventListener("slotchange", () => {
        requestAnimationFrame(() => this.updatePositions());
      });
    }
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    if (this.rafId) cancelAnimationFrame(this.rafId);
    this.scrollContainer?.removeEventListener("scroll", this.handleScroll);
    window.removeEventListener("resize", this.updatePositions);
  }

  private setupScrollTracking() {
    // Listen on window for page scroll
    window.addEventListener("scroll", this.handleScroll, { passive: true });
    window.addEventListener("resize", this.updatePositions);

    // Initial position calculation after a short delay to let content render
    setTimeout(() => this.updatePositions(), 100);
  }

  private handleScroll = () => {
    if (this.rafId) return;

    this.rafId = requestAnimationFrame(() => {
      this.rafId = null;
      this.updateActiveIndicator();
    });
  };

  private updatePositions = () => {
    // Query slotted content - need to get assigned elements from the slot
    const slot = this.shadowRoot?.querySelector("slot");
    const slottedElements = slot?.assignedElements({ flatten: true }) || [];

    const positions: ItemPosition[] = [];

    slottedElements.forEach((el) => {
      // Check if this element has data-content-id or find children with it
      const cards = el.matches("[data-content-id]")
        ? [el]
        : Array.from(el.querySelectorAll("[data-content-id]"));

      cards.forEach((card) => {
        const idStr = card.getAttribute("data-content-id");
        if (!idStr) return;
        const id = parseInt(idStr, 10);
        if (Number.isNaN(id)) return;

        const rect = card.getBoundingClientRect();
        const scrollTop = window.scrollY || document.documentElement.scrollTop;

        positions.push({
          id,
          top: rect.top + scrollTop,
          height: rect.height,
        });
      });
    });

    this.itemPositions = positions;
    this.updateActiveIndicator();
  };

  private updateActiveIndicator() {
    const scrollTop = window.scrollY || document.documentElement.scrollTop;
    const viewportCenter = scrollTop + window.innerHeight / 3; // Upper third of viewport

    // Find the item closest to viewport center
    let closestItem: ItemPosition | null = null;
    let closestDistance = Infinity;

    for (const pos of this.itemPositions) {
      const itemCenter = pos.top + pos.height / 2;
      const distance = Math.abs(itemCenter - viewportCenter);

      if (distance < closestDistance) {
        closestDistance = distance;
        closestItem = pos;
      }
    }

    if (closestItem && closestItem.id !== this.activeItemId) {
      this.activeItemId = closestItem.id;
      this.dispatchEvent(
        new CustomEvent("active-item-change", {
          detail: { itemId: this.activeItemId },
          bubbles: true,
        })
      );
    }
  }

  scrollToItem(itemId: number) {
    // Query slotted content for the card
    const slot = this.shadowRoot?.querySelector("slot");
    const slottedElements = slot?.assignedElements({ flatten: true }) || [];

    for (const el of slottedElements) {
      const card = el.matches(`[data-content-id="${itemId}"]`)
        ? el
        : el.querySelector(`[data-content-id="${itemId}"]`);

      if (card) {
        card.scrollIntoView({ behavior: "smooth", block: "center" });
        return;
      }
    }
  }

  updated(changedProperties: Map<string, unknown>) {
    if (changedProperties.has("content")) {
      // Recalculate positions when content changes
      requestAnimationFrame(() => this.updatePositions());
    }
  }

  private handleDotClick(itemId: number) {
    // Immediately set active for instant feedback
    this.activeItemId = itemId;
    this.scrollToItem(itemId);
  }

  render() {
    return html`
      <div class="flex gap-8 max-w-5xl mx-auto">
        <!-- Fixed Timeline Rail -->
        <div class="w-16 shrink-0">
          <div class="sticky top-24">
            <!-- Timeline container with line -->
            <div class="relative flex flex-col items-center py-2">
              <!-- Vertical line running full height -->
              <div
                class="absolute left-1/2 top-0 bottom-0 w-px bg-base-300 -translate-x-1/2"
              ></div>

              <!-- Section labels and dots -->
              ${this.sections.map(
                (section) => html`
                  <div class="relative z-10 flex flex-col items-center">
                    <!-- Section label -->
                    <div
                      class="text-[10px] font-bold uppercase tracking-wider text-base-content/50 mb-2 px-1 py-0.5 bg-base-100 whitespace-nowrap"
                    >
                      ${section.label}
                    </div>

                    <!-- Dots for this section's items -->
                    ${section.items.map(
                      (item) => html`
                        <button
                          class="relative w-2.5 h-2.5 my-2.5 rounded-full transition-all duration-200 group
                            ${this.activeItemId === (item.type === 'tweet' ? item.id : item.thread.id)
                            ? "bg-primary scale-150 ring-4 ring-primary/20"
                            : "bg-base-300 hover:bg-primary/70 hover:scale-125"}"
                          @click=${() => this.handleDotClick(item.type === 'tweet' ? item.id : item.thread.id)}
                          title="${formatTimeOfDay(item.type === 'tweet' ? item.created_at : item.thread.created_at)}"
                        >
                          <!-- Tooltip on hover -->
                          <div
                            class="absolute left-full ml-3 top-1/2 -translate-y-1/2 opacity-0 group-hover:opacity-100
                              transition-opacity pointer-events-none whitespace-nowrap bg-base-100 border border-base-300
                              px-2 py-1 rounded-lg shadow-lg text-xs z-20"
                          >
                            ${item.type === 'thread' ? 'Thread · ' : ''}${formatTimeOfDay(item.type === 'tweet' ? item.created_at : item.thread.created_at)}
                          </div>
                        </button>
                      `
                    )}

                    <!-- Spacer between sections -->
                    <div class="h-4"></div>
                  </div>
                `
              )}
            </div>
          </div>
        </div>

        <!-- Content Area -->
        <div class="flex-1 min-w-0">
          <!-- Header -->
          <div class="flex justify-between items-center mb-6 pb-4 border-b border-base-200">
            <div>
              <h1 class="text-xl font-semibold text-base-content">Queue</h1>
              <p class="text-sm text-base-content/50">${this.content.length} items pending</p>
            </div>
            <button class="btn btn-ghost btn-sm gap-2" @click=${() => this.dispatchEvent(new CustomEvent('refresh', { bubbles: true }))}>
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
              </svg>
              Refresh
            </button>
          </div>
          <slot></slot>
        </div>
      </div>
    `;
  }
}
