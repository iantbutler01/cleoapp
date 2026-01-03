import { LitElement, html } from "lit";
import { customElement, state } from "lit/decorators.js";
import { api, User, ContentItem } from "../api";
import { tailwindStyles } from "../styles/shared";
import { groupContentByTime, TimelineSection } from "../utils/time-grouping";
import "./tweet-card";
import "./thread-card";

@customElement("dashboard-page")
export class DashboardPage extends LitElement {
  static styles = [tailwindStyles];

  @state() user: User | null = null;
  @state() content: ContentItem[] = [];
  @state() sections: TimelineSection[] = [];
  @state() loading = true;
  @state() loadError: string | null = null;
  @state() apiToken: string | null = null;
  @state() showTokenModal = false;
  @state() generatingToken = false;
  @state() tokenError: string | null = null;
  @state() loadingToken = false;
  @state() logoutError: string | null = null;
  @state() loggingOut = false;
  @state() activeItemId: number | null = null;

  private scrollContainer: HTMLElement | null = null;
  private rafId: number | null = null;

  async connectedCallback() {
    super.connectedCallback();
    await this.loadData();
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    if (this.rafId) cancelAnimationFrame(this.rafId);
    this.scrollContainer?.removeEventListener("scroll", this.handleScroll);
  }

  protected updated(changedProperties: Map<string, unknown>) {
    super.updated(changedProperties);

    // Set up scroll tracking once content is loaded
    if (
      changedProperties.has("content") &&
      this.content.length > 0 &&
      !this.scrollContainer
    ) {
      const container = this.shadowRoot?.querySelector(".content-scroll");
      if (container instanceof HTMLElement) {
        this.scrollContainer = container;
        this.scrollContainer.addEventListener("scroll", this.handleScroll, {
          passive: true,
        });
      }
    }

    // Update active item after content changes
    if (changedProperties.has("content") && this.content.length > 0) {
      // Set first item as active if none selected
      if (this.activeItemId === null) {
        const firstItem = this.content[0];
        this.activeItemId =
          firstItem.type === "tweet" ? firstItem.id : firstItem.thread.id;
      }
      // Recalculate after DOM updates
      requestAnimationFrame(() => this.updateActiveItem());
    }
  }

  private handleScroll = () => {
    if (this.rafId) return;
    this.rafId = requestAnimationFrame(() => {
      this.rafId = null;
      this.updateActiveItem();
    });
  };

  private updateActiveItem() {
    if (!this.scrollContainer) return;

    const cards = this.shadowRoot?.querySelectorAll("[data-content-id]");
    if (!cards?.length) return;

    const containerRect = this.scrollContainer.getBoundingClientRect();
    const targetY = containerRect.top + containerRect.height / 3;

    let closestId: number | null = null;
    let closestDistance = Infinity;

    cards.forEach((card) => {
      const rect = card.getBoundingClientRect();
      const cardCenter = rect.top + rect.height / 2;
      const distance = Math.abs(cardCenter - targetY);

      if (distance < closestDistance) {
        closestDistance = distance;
        const idAttr = card.getAttribute("data-content-id");
        closestId = idAttr ? parseInt(idAttr, 10) : null;
      }
    });

    if (closestId !== null && !Number.isNaN(closestId) && closestId !== this.activeItemId) {
      this.activeItemId = closestId;
    }
  }

  private scrollToItem(itemId: number) {
    const card = this.shadowRoot?.querySelector(
      `[data-content-id="${itemId}"]`
    );
    if (card) {
      card.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  }

  private handleDotClick(itemId: number) {
    this.activeItemId = itemId;
    this.scrollToItem(itemId);
  }

  async loadData() {
    this.loading = true;
    this.loadError = null;
    try {
      const [user, contentResponse] = await Promise.all([
        api.getMe(),
        api.getContent({ platform: "twitter", status: "pending" }),
      ]);
      this.user = user;
      this.content = contentResponse.items;
      this.sections = groupContentByTime(this.content);
    } catch (e) {
      console.error("Failed to load data:", e);
      this.loadError = e instanceof Error ? e.message : "Failed to load data";
    } finally {
      this.loading = false;
    }
  }

  async handleLogout() {
    this.loggingOut = true;
    this.logoutError = null;

    try {
      await api.logout();
      this.dispatchEvent(new CustomEvent("logout"));
    } catch (e) {
      console.error("Failed to logout:", e);
      this.logoutError = e instanceof Error ? e.message : "Failed to logout";
    } finally {
      this.loggingOut = false;
    }
  }

  clearLogoutError() {
    this.logoutError = null;
  }

  handleContentAction() {
    this.loadData();
  }

  async openTokenModal() {
    this.showTokenModal = true;
    this.loadingToken = true;
    this.tokenError = null;

    try {
      this.apiToken = await api.getApiToken();
    } catch (e) {
      console.error("Failed to get token:", e);
      this.tokenError = e instanceof Error ? e.message : "Failed to load token";
    } finally {
      this.loadingToken = false;
    }
  }

  closeTokenModal() {
    this.showTokenModal = false;
  }

  async generateToken() {
    this.generatingToken = true;
    this.tokenError = null;
    try {
      const result = await api.generateApiToken();
      this.apiToken = result.api_token;
    } catch (e) {
      console.error("Failed to generate token:", e);
      this.tokenError = e instanceof Error ? e.message : "Failed to generate token";
    } finally {
      this.generatingToken = false;
    }
  }

  copyToken() {
    if (this.apiToken) {
      navigator.clipboard.writeText(this.apiToken);
    }
  }

  handleMediaChanged() {
    // When media is added/removed, card height changes significantly.
    // The tweet-content component has a loading state that causes layout shifts.
    // Save scroll position and restore after media finishes loading.
    const scrollTop = this.scrollContainer?.scrollTop ?? 0;

    const restoreScroll = () => {
      if (this.scrollContainer && scrollTop > 0) {
        this.scrollContainer.scrollTop = scrollTop;
      }
      this.updateActiveItem();
    };

    // Immediate restore attempt
    requestAnimationFrame(restoreScroll);
    // Delayed restore after media likely finishes loading
    setTimeout(restoreScroll, 300);
  }

  renderContentItem(item: ContentItem) {
    const itemId = item.type === "tweet" ? item.id : item.thread.id;
    if (item.type === "tweet") {
      return html`
        <div data-content-id=${itemId}>
          <tweet-card
            .tweet=${item}
            @tweet-posted=${this.handleContentAction}
            @tweet-dismissed=${this.handleContentAction}
            @collateral-updated=${this.handleMediaChanged}
          ></tweet-card>
        </div>
      `;
    } else {
      return html`
        <div data-content-id=${itemId}>
          <thread-card
            .thread=${item}
            @thread-posted=${this.handleContentAction}
            @thread-deleted=${this.handleContentAction}
            @thread-updated=${this.handleContentAction}
          ></thread-card>
        </div>
      `;
    }
  }

  render() {
    if (this.loading) {
      return html`
        <div class="flex justify-center items-center min-h-screen">
          <span class="loading loading-spinner loading-lg"></span>
        </div>
      `;
    }

    if (this.loadError) {
      return html`
        <div class="flex justify-center items-center min-h-screen">
          <div class="card bg-base-100 shadow-xl max-w-md">
            <div class="card-body items-center text-center">
              <svg class="w-12 h-12 text-error" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/>
              </svg>
              <h2 class="card-title text-error">Failed to load</h2>
              <p class="text-base-content/70">${this.loadError}</p>
              <div class="card-actions mt-4">
                <button class="btn btn-primary" @click=${this.loadData}>Try Again</button>
              </div>
            </div>
          </div>
        </div>
      `;
    }

    return html`
      <div class="min-h-screen bg-base-200/30">
        <!-- Navbar -->
        <div class="navbar bg-base-100 border-b border-base-200 px-6">
          <div class="flex-1">
            <span class="text-lg font-semibold tracking-tight">Cleo</span>
          </div>
          <div class="flex-none gap-2">
            <div class="dropdown dropdown-end">
              <div
                tabindex="0"
                role="button"
                class="btn btn-ghost btn-sm gap-2"
              >
                <div
                  class="w-7 h-7 rounded-full bg-primary/10 flex items-center justify-center"
                >
                  <span class="text-sm font-medium text-primary"
                    >${this.user?.username?.charAt(0).toUpperCase() ||
                    "?"}</span
                  >
                </div>
                <span class="text-sm">@${this.user?.username}</span>
                <svg
                  class="w-4 h-4 opacity-50"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    stroke-width="2"
                    d="M19 9l-7 7-7-7"
                  />
                </svg>
              </div>
              <ul
                tabindex="0"
                class="menu menu-sm dropdown-content bg-base-100 rounded-lg z-10 mt-2 w-48 p-1 shadow-lg border border-base-200"
              >
                <li>
                  <a @click=${this.openTokenModal} class="rounded-md"
                    >API Token</a
                  >
                </li>
                <li>
                  <a
                    @click=${this.handleLogout}
                    class="rounded-md text-error ${this.loggingOut ? 'pointer-events-none opacity-50' : ''}"
                  >
                    ${this.loggingOut
                      ? html`<span class="loading loading-spinner loading-xs"></span> Logging out...`
                      : "Logout"}
                  </a>
                </li>
              </ul>
            </div>
          </div>
        </div>

        ${this.logoutError ? html`
          <div class="alert alert-error mx-6 mt-2">
            <svg xmlns="http://www.w3.org/2000/svg" class="stroke-current shrink-0 h-5 w-5" fill="none" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
            <span>${this.logoutError}</span>
            <button class="btn btn-ghost btn-xs" @click=${this.clearLogoutError}>Dismiss</button>
          </div>
        ` : ''}

        <!-- 3-Panel Layout -->
        <div class="flex h-[calc(100vh-65px)]">
          <!-- Left Sidebar - Floating Timeline -->
          <div class="w-1/5 shrink-0 flex flex-col items-center pt-20 pl-4">
            <!-- Timeline dots - floating alongside content -->
            <div
              class="sticky top-24 flex flex-col items-center bg-base-100/80 backdrop-blur-sm rounded-2xl py-4 px-3 border border-base-300/30 shadow-sm"
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
                      return html`
                        <button
                          class="w-2 h-2 my-2 rounded-full transition-all
                          ${isActive
                            ? "bg-primary scale-150 ring-4 ring-primary/20"
                            : "bg-base-300 hover:bg-primary/70 hover:scale-125"}"
                          title="${item.type === "tweet"
                            ? item.created_at
                            : item.thread.created_at}"
                          @click=${() => this.handleDotClick(itemId)}
                        ></button>
                      `;
                    })}
                    <div class="h-3"></div>
                  </div>
                `
              )}
            </div>
          </div>

          <!-- Center - Content Feed -->
          <div class="content-scroll flex-1 overflow-y-auto py-6 px-8">
            ${this.content.length === 0
              ? html`
                  <div class="max-w-xl mx-auto">
                    <div class="card bg-base-100 rounded-2xl">
                      <div class="card-body items-center text-center">
                        <svg
                          class="w-12 h-12 opacity-30"
                          fill="none"
                          stroke="currentColor"
                          viewBox="0 0 24 24"
                        >
                          <path
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            stroke-width="1"
                            d="M20 13V6a2 2 0 00-2-2H6a2 2 0 00-2 2v7m16 0v5a2 2 0 01-2 2H6a2 2 0 01-2-2v-5m16 0h-2.586a1 1 0 00-.707.293l-2.414 2.414a1 1 0 01-.707.293h-3.172a1 1 0 01-.707-.293l-2.414-2.414A1 1 0 006.586 13H4"
                          />
                        </svg>
                        <h2 class="card-title mt-3 text-base">
                          No pending content
                        </h2>
                        <p class="opacity-50 text-sm">
                          New suggestions will appear here when the AI finds
                          interesting moments.
                        </p>
                      </div>
                    </div>
                  </div>
                `
              : html`
                  <!-- Header -->
                  <div class="max-w-xl mx-auto mb-6">
                    <div class="flex justify-between items-center">
                      <div>
                        <h1 class="text-lg font-semibold text-base-content">
                          Queue
                        </h1>
                        <p class="text-xs text-base-content/50">
                          ${this.content.length} items pending
                        </p>
                      </div>
                      <button
                        class="btn btn-ghost btn-sm gap-1.5"
                        @click=${this.loadData}
                      >
                        <svg
                          class="w-3.5 h-3.5"
                          fill="none"
                          stroke="currentColor"
                          viewBox="0 0 24 24"
                        >
                          <path
                            stroke-linecap="round"
                            stroke-linejoin="round"
                            stroke-width="2"
                            d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
                          />
                        </svg>
                        Refresh
                      </button>
                    </div>
                  </div>
                  <!-- Cards -->
                  <div class="space-y-4 flex flex-col items-center">
                    ${this.content.map((item) => this.renderContentItem(item))}
                  </div>
                `}
          </div>

          <!-- Right Sidebar - Context Panel -->
          <div
            class="w-64 shrink-0 border-l border-base-300/30 bg-base-100/50 p-4 overflow-y-auto"
          >
            <!-- Stats -->
            <div class="mb-6">
              <h3
                class="text-xs font-semibold uppercase tracking-wider text-base-content/40 mb-3"
              >
                Today
              </h3>
              <div class="grid grid-cols-2 gap-3">
                <div class="bg-base-200/50 rounded-xl p-3">
                  <div class="text-2xl font-bold text-primary">
                    ${this.content.filter((c) => c.type === "tweet").length}
                  </div>
                  <div class="text-xs text-base-content/50">Tweets</div>
                </div>
                <div class="bg-base-200/50 rounded-xl p-3">
                  <div class="text-2xl font-bold text-primary">
                    ${this.content.filter((c) => c.type === "thread").length}
                  </div>
                  <div class="text-xs text-base-content/50">Threads</div>
                </div>
              </div>
            </div>

            <!-- Quick Actions -->
            <div class="mb-6">
              <h3
                class="text-xs font-semibold uppercase tracking-wider text-base-content/40 mb-3"
              >
                Quick Actions
              </h3>
              <div class="space-y-2">
                <button
                  class="btn btn-ghost btn-sm w-full justify-start gap-2 text-base-content/70"
                >
                  <svg
                    class="w-4 h-4"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      stroke-width="2"
                      d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"
                    />
                  </svg>
                  Post all pending
                </button>
                <button
                  class="btn btn-ghost btn-sm w-full justify-start gap-2 text-base-content/70"
                >
                  <svg
                    class="w-4 h-4"
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      stroke-width="2"
                      d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
                    />
                  </svg>
                  Clear dismissed
                </button>
              </div>
            </div>

            <!-- Recent Activity (placeholder) -->
            <div>
              <h3
                class="text-xs font-semibold uppercase tracking-wider text-base-content/40 mb-3"
              >
                Recent Posts
              </h3>
              <div class="text-sm text-base-content/40 text-center py-4">
                No recent activity
              </div>
            </div>
          </div>
        </div>

        <!-- API Token Modal -->
        ${this.showTokenModal
          ? html`
              <div class="modal modal-open">
                <div class="modal-box">
                  <h3 class="font-bold text-lg">API Token</h3>
                  <p class="py-2 text-sm opacity-70">
                    Use this token to authenticate the Cleo daemon on your
                    machine.
                  </p>

                  ${this.tokenError
                    ? html`
                        <div class="alert alert-error mt-4">
                          <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/>
                          </svg>
                          <span>${this.tokenError}</span>
                        </div>
                      `
                    : ""}

                  ${this.loadingToken
                    ? html`
                        <div class="mt-4 text-center py-4">
                          <span class="loading loading-spinner loading-md"></span>
                        </div>
                      `
                    : this.apiToken
                    ? html`
                        <div class="form-control mt-4">
                          <div class="join w-full">
                            <input
                              type="text"
                              readonly
                              value=${this.apiToken}
                              class="input input-bordered join-item flex-1 font-mono text-sm"
                            />
                            <button
                              class="btn join-item"
                              @click=${this.copyToken}
                            >
                              Copy
                            </button>
                          </div>
                        </div>
                      `
                    : html`
                        <div class="mt-4 text-center py-4">
                          <p class="opacity-70">No token generated yet</p>
                        </div>
                      `}

                  <div class="modal-action">
                    <button
                      class="btn btn-primary"
                      @click=${this.generateToken}
                      ?disabled=${this.generatingToken || this.loadingToken}
                    >
                      ${this.generatingToken
                        ? html`<span
                            class="loading loading-spinner loading-sm"
                          ></span>`
                        : this.apiToken
                        ? "Regenerate"
                        : "Generate Token"}
                    </button>
                    <button class="btn" @click=${this.closeTokenModal}>
                      Close
                    </button>
                  </div>
                </div>
                <div
                  class="modal-backdrop"
                  @click=${this.closeTokenModal}
                ></div>
              </div>
            `
          : ""}
      </div>
    `;
  }
}
