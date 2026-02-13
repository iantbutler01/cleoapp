import { LitElement, html } from "lit";
import { customElement, state } from "lit/decorators.js";
import { api, User, ContentItem } from "../api";
import { tailwindStyles } from "../styles/shared";
import "./tweet-card";
import "./thread-card";
import "./sidebar-toolbar";
import "./nudges-modal";

@customElement("dashboard-page")
export class DashboardPage extends LitElement {
  static styles = [tailwindStyles];

  @state() user: User | null = null;
  @state() content: ContentItem[] = [];
  @state() loading = true;
  @state() refreshing = false;
  @state() loadError: string | null = null;
  @state() apiToken: string | null = null;
  @state() showTokenModal = false;
  @state() generatingToken = false;
  @state() tokenError: string | null = null;
  @state() loadingToken = false;
  @state() logoutError: string | null = null;
  @state() loggingOut = false;
  @state() currentIndex = 0;
  @state() viewMode: "queue" | "sent" = "queue";
  @state() showNudgesModal = false;
  @state() notificationPermission: NotificationPermission | "unsupported" = "unsupported";

  private readonly pollIntervalMs = 45_000;
  private pollTimer: number | null = null;
  private pushSubscriptionInitialized = false;
  private pushSetupInProgress = false;
  private queueItemKeys = new Set<string>();
  private queueInitialized = false;

  async connectedCallback() {
    super.connectedCallback();
    document.addEventListener("keydown", this.handleGlobalKeydown);
    document.addEventListener("visibilitychange", this.handleVisibilityChange);
    this.updateNotificationPermission();
    this.startPolling();
    const urlParams = new URLSearchParams(window.location.search);
    const requestedView = urlParams.get("view");
    if (requestedView === "queue" || requestedView === "sent") {
      this.viewMode = requestedView;
    }
    await this.loadData();
    if (this.notificationPermission === "granted") {
      void this.setupPushNotifications();
    }
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    document.removeEventListener("keydown", this.handleGlobalKeydown);
    document.removeEventListener("visibilitychange", this.handleVisibilityChange);
    this.stopPolling();
  }

  private handleGlobalKeydown = (e: KeyboardEvent) => {
    // Cmd+. or Ctrl+. to open nudges modal
    if ((e.metaKey || e.ctrlKey) && e.key === ".") {
      e.preventDefault();
      this.showNudgesModal = true;
    }
  };

  private handleVisibilityChange = () => {
    if (document.visibilityState === "visible" && this.viewMode === "queue") {
      this.loadData();
    }
  };

  private updateNotificationPermission() {
    if (!("Notification" in window)) {
      this.notificationPermission = "unsupported";
      return;
    }
    this.notificationPermission = Notification.permission;
  }

  private startPolling() {
    this.stopPolling();
    this.pollTimer = window.setInterval(() => {
      if (document.visibilityState === "hidden" && this.viewMode === "queue") {
        this.loadData();
      }
    }, this.pollIntervalMs);
  }

  private stopPolling() {
    if (this.pollTimer !== null) {
      clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
  }

  private getItemKey(item: ContentItem): string {
    return `${item.type}-${item.id}`;
  }

  private async processQueueNotifications(items: ContentItem[]) {
    if (this.viewMode !== "queue" || this.notificationPermission !== "granted" || !document.hidden) {
      this.queueItemKeys = new Set(items.map((item) => this.getItemKey(item)));
      this.queueInitialized = true;
      return;
    }

    const nextKeys = new Set(items.map((item) => this.getItemKey(item)));
    const newItems = items.filter((item) => !this.queueItemKeys.has(this.getItemKey(item)));

    if (!this.queueInitialized) {
      this.queueItemKeys = nextKeys;
      this.queueInitialized = true;
      return;
    }

    if (newItems.length > 0) {
      const maxNotifications = 3;
      for (const item of newItems.slice(0, maxNotifications)) {
        await this.showContentNotification(item);
      }

      const remaining = newItems.length - maxNotifications;
      if (remaining > 0) {
        await this.showSummaryNotification(remaining);
      }
    }

    this.queueItemKeys = nextKeys;
  }

  private async requestNotificationPermission() {
    if (!("Notification" in window)) {
      this.notificationPermission = "unsupported";
      return;
    }

    const permission = await Notification.requestPermission();
    this.notificationPermission = permission;
    if (permission === "granted") {
      await this.setupPushNotifications();
    }
  }

  private async setupPushNotifications() {
    if (this.pushSetupInProgress) {
      return;
    }

    if (!("serviceWorker" in navigator) || !("PushManager" in window) || this.notificationPermission !== "granted") {
      return;
    }

    const maybeServiceWorker = await navigator.serviceWorker.ready;
    this.pushSetupInProgress = true;

    try {
      const existing = await maybeServiceWorker.pushManager.getSubscription();
      const payload = await api.getVapidPublicKey();
      const appKey = this.urlBase64ToUint8Array(payload);

      const subscription = (existing && this.applicationServerKeyMatches(existing, appKey))
        ? existing
        : await this.recreateSubscription(maybeServiceWorker, existing, appKey);

      if (!subscription) {
        throw new Error("Failed to create push subscription");
      }

      const json = subscription.toJSON() as {
        endpoint?: string;
        keys?: { p256dh?: string; auth?: string };
      };

      if (!json.endpoint || !json.keys?.p256dh || !json.keys?.auth) {
        throw new Error("Missing endpoint or keys from push subscription");
      }

      await api.savePushSubscription({
        endpoint: json.endpoint,
        keys: {
          p256dh: json.keys.p256dh,
          auth: json.keys.auth,
        },
      });

      this.pushSubscriptionInitialized = true;
    } catch (error) {
      console.error("Failed to configure push notifications:", error);
      this.pushSubscriptionInitialized = false;
    } finally {
      this.pushSetupInProgress = false;
    }
  }

  private async recreateSubscription(
    serviceWorker: ServiceWorkerRegistration,
    existing: PushSubscription | null,
    applicationServerKey: Uint8Array,
  ): Promise<PushSubscription | null> {
    try {
      if (existing) {
        await existing.unsubscribe();
      }

      return await serviceWorker.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey,
      });
    } catch {
      return null;
    }
  }

  private applicationServerKeyMatches(existing: PushSubscription, appKey: Uint8Array): boolean {
    const existingAppKey = existing.options.applicationServerKey;
    if (!existingAppKey) {
      return false;
    }

    const existingKeyBytes = existingAppKey instanceof ArrayBuffer
      ? new Uint8Array(existingAppKey)
      : typeof existingAppKey === "string"
        ? this.urlBase64ToUint8Array(existingAppKey)
        : existingAppKey instanceof Uint8Array
          ? existingAppKey
          : new Uint8Array(existingAppKey as ArrayBufferLike);

    if (existingKeyBytes.length !== appKey.length) {
      return false;
    }

    return existingKeyBytes.every((value, index) => value === appKey[index]);
  }

  private urlBase64ToUint8Array(value: string): Uint8Array {
    const padding = "=".repeat((4 - (value.length % 4)) % 4);
    const base64 = (value + padding).replace(/-/g, "+").replace(/_/g, "/");
    const rawData = atob(base64);
    const outputArray = new Uint8Array(rawData.length);

    for (let i = 0; i < rawData.length; i++) {
      outputArray[i] = rawData.charCodeAt(i);
    }

    return outputArray;
  }

  private async showSummaryNotification(remaining: number) {
    if (remaining <= 0) return;
    await this.dispatchNotification(
      "More content ready",
      `${remaining} additional items are ready to review in Cleo.`,
    );
  }

  private async showContentNotification(item: ContentItem) {
    if (item.type === "tweet") {
      await this.dispatchNotification("New tweet ready", item.text, item.id, item.type);
      return;
    }

    const threadTitle = item.thread.title ?? `Thread (${item.tweets.length} tweets)`;
    await this.dispatchNotification("New thread ready", threadTitle, item.thread.id, item.type);
  }

  private async dispatchNotification(title: string, body: string, id?: number, type?: string) {
    const tag = id !== undefined && type ? `cleo-${type}-${id}` : "cleo-content";
    const options: NotificationOptions = {
      body,
      icon: "/icon-192.png",
      badge: "/icon-192.png",
      tag,
      data: {
        url: "/?view=queue",
        type,
        id,
      },
      timestamp: Date.now(),
      renotify: true,
      requireInteraction: false,
      vibrate: [80, 40, 80],
    };

    if (!("Notification" in window) || Notification.permission !== "granted") {
      return;
    }

    try {
      const registration = await navigator.serviceWorker.getRegistration();
      if (registration) {
        await registration.showNotification(title, options);
        return;
      }
    } catch {
      // Intentionally ignore and fall back to window notification.
    }

    try {
      new Notification(title, options);
    } catch (error) {
      console.error("Failed to show notification:", error);
    }
  }

  async loadData() {
    // Use refreshing state if we already have content (keeps UI stable)
    this.updateNotificationPermission();
    const isRefresh = this.content.length > 0;
    if (isRefresh) {
      this.refreshing = true;
    } else {
      this.loading = true;
    }
    this.loadError = null;

    // Minimum spinner display time to avoid flash
    const minDisplayTime = 400;
    const startTime = Date.now();

    try {
      const [user, contentResponse] = await Promise.all([
        api.getMe(),
        api.getContent({
          platform: "twitter",
          status: this.viewMode === "queue" ? "pending" : "posted",
        }),
      ]);
      this.user = user;
      if (this.viewMode === "queue") {
        await this.processQueueNotifications(contentResponse.items);
      }
      this.content = contentResponse.items;
      this.currentIndex = 0;
    } catch (e) {
      console.error("Failed to load data:", e);
      this.loadError = e instanceof Error ? e.message : "Failed to load data";
    } finally {
      // Wait for minimum display time before hiding spinner
      const elapsed = Date.now() - startTime;
      if (elapsed < minDisplayTime) {
        await new Promise((resolve) =>
          setTimeout(resolve, minDisplayTime - elapsed),
        );
      }
      this.loading = false;
      this.refreshing = false;
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

  private setViewMode(mode: "queue" | "sent") {
    if (this.viewMode === mode) return;
    this.viewMode = mode;
    if (mode === "queue") {
      this.queueInitialized = false;
    }
    this.loadData();
  }

  private handleItemPosted() {
    this.content = this.content.filter((_, i) => i !== this.currentIndex);
    if (this.currentIndex >= this.content.length) {
      this.currentIndex = Math.max(0, this.content.length - 1);
    }
  }

  private handleItemDismissed() {
    this.handleItemPosted();
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
      this.tokenError =
        e instanceof Error ? e.message : "Failed to generate token";
    } finally {
      this.generatingToken = false;
    }
  }

  copyToken() {
    if (this.apiToken) {
      navigator.clipboard.writeText(this.apiToken);
    }
  }

  private renderCurrentItem(item: ContentItem) {
    if (item.type === "tweet") {
      return html`
        <tweet-card
          .tweet=${item}
          @tweet-posted=${this.handleItemPosted}
          @tweet-dismissed=${this.handleItemDismissed}
        ></tweet-card>
      `;
    }
    return html`
      <thread-card
        .thread=${item}
        @thread-posted=${this.handleItemPosted}
        @thread-deleted=${this.handleItemDismissed}
      ></thread-card>
    `;
  }

  render() {
    if (this.loadError) {
      return html`
        <div class="flex justify-center items-center min-h-screen">
          <div class="card bg-base-100 shadow-xl max-w-md">
            <div class="card-body items-center text-center">
              <svg
                class="w-12 h-12 text-error"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  stroke-width="2"
                  d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
                />
              </svg>
              <h2 class="card-title text-error">Failed to load</h2>
              <p class="text-base-content/70">${this.loadError}</p>
              <div class="card-actions mt-4">
                <button class="btn btn-primary" @click=${this.loadData}>
                  Try Again
                </button>
              </div>
            </div>
          </div>
        </div>
      `;
    }

    return html`
      <div class="min-h-screen bg-base-200/30">
        <!-- Navbar -->
        <div class="navbar bg-base-100 border-b border-base-200 px-4 sm:px-6">
          <div class="flex-1 min-w-0">
            <span class="text-lg sm:text-xl font-semibold tracking-tight truncate"
              >Cleo</span
            >
          </div>
          <div class="flex-none gap-2">
            <button
              class="btn btn-ghost btn-sm lg:hidden"
              @click=${() => (this.showNudgesModal = true)}
            >
              Nudges
            </button>
            <button
              class="btn btn-ghost btn-sm lg:hidden"
              @click=${() => this.requestNotificationPermission()}
            >
              Notify
            </button>
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
                  <a
                    @click=${() => (this.showNudgesModal = true)}
                    class="rounded-md"
                    >Voice & Style</a
                  >
                </li>
                <li>
                  <a
                    @click=${() => this.requestNotificationPermission()}
                    class="rounded-md ${this.notificationPermission === "granted"
                      ? "text-success"
                      : this.notificationPermission === "unsupported"
                        ? "pointer-events-none opacity-50"
                        : ""}"
                  >
                    ${this.notificationPermission === "granted"
                      ? "Notifications enabled"
                      : this.notificationPermission === "denied"
                        ? "Notifications blocked"
                        : "Enable notifications"}
                  </a>
                </li>
                <li>
                  <a @click=${this.openTokenModal} class="rounded-md"
                    >API Token</a
                  >
                </li>
                <li>
                  <a
                    @click=${this.handleLogout}
                    class="rounded-md text-error ${this.loggingOut
                      ? "pointer-events-none opacity-50"
                      : ""}"
                  >
                    ${this.loggingOut
                      ? html`<span
                            class="loading loading-spinner loading-xs"
                          ></span>
                          Logging out...`
                      : "Logout"}
                  </a>
                </li>
              </ul>
            </div>
          </div>
        </div>

        ${this.logoutError
          ? html`
              <div class="alert alert-error mx-6 mt-2">
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  class="stroke-current shrink-0 h-5 w-5"
                  fill="none"
                  viewBox="0 0 24 24"
                >
                  <path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    stroke-width="2"
                    d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z"
                  />
                </svg>
                <span>${this.logoutError}</span>
                <button
                  class="btn btn-ghost btn-xs"
                  @click=${this.clearLogoutError}
                >
                  Dismiss
                </button>
              </div>
            `
          : ""}

        <!-- Stack-Based Layout (Grid + Scroll Container) -->
        <div class="flex flex-col lg:flex-row h-auto lg:h-[calc(100vh-65px)]">
          <!-- Left Sidebar - Toolbar -->
          <div class="hidden lg:flex shrink-0 items-start pt-20 pl-4">
            <sidebar-toolbar
              @open-nudges=${() => (this.showNudgesModal = true)}
            ></sidebar-toolbar>
          </div>

          <!-- Center - Single Item View -->
          <div
            class="flex-1 flex flex-col items-center justify-start min-h-[calc(100vh-65px)] p-4 sm:p-6 lg:p-8 relative"
          >
            ${this.loading || this.refreshing
              ? html`
                  <div
                    class="absolute inset-0 z-10 flex justify-center items-center bg-base-200/60 backdrop-blur-sm animate-in fade-in duration-150"
                  >
                    <span class="loading loading-spinner loading-lg"></span>
                  </div>
                `
              : ""}

            <div class="w-full max-w-2xl flex flex-wrap gap-2 mb-6 mt-6">
              <button
                class="btn btn-sm ${this.viewMode === "queue"
                  ? "btn-primary"
                  : "btn-ghost"}"
                @click=${() => this.setViewMode("queue")}
              >
                Queue${this.viewMode === "queue"
                  ? ` (${this.content.length})`
                  : ""}
              </button>
              <button
                class="btn btn-sm ${this.viewMode === "sent"
                  ? "btn-primary"
                  : "btn-ghost"}"
                @click=${() => this.setViewMode("sent")}
              >
                Sent${this.viewMode === "sent"
                  ? ` (${this.content.length})`
                  : ""}
              </button>
              <button class="btn btn-sm btn-ghost" @click=${this.loadData}>
                Refresh
              </button>
            </div>

            ${this.content.length > 0
              ? html`
                  <div class="w-full max-w-2xl">
                    ${this.renderCurrentItem(this.content[this.currentIndex])}
                  </div>
                  <div class="flex items-center gap-4 mt-6">
                    <button
                      class="btn btn-circle btn-ghost"
                      ?disabled=${this.currentIndex === 0}
                      @click=${() => this.currentIndex--}
                    >
                      ←
                    </button>
                    <span class="text-sm opacity-60">
                      ${this.currentIndex + 1} / ${this.content.length}
                    </span>
                    <button
                      class="btn btn-circle btn-ghost"
                      ?disabled=${this.currentIndex >= this.content.length - 1}
                      @click=${() => this.currentIndex++}
                    >
                      →
                    </button>
                  </div>
                `
              : html`
                  <div class="text-center opacity-50">
                    No items in ${this.viewMode}
                  </div>
                `}
          </div>

          <!-- Right Sidebar - Context Panel -->
          <div
            class="hidden lg:block w-64 shrink-0 border-l border-base-300/30 bg-base-100/50 p-4 overflow-y-auto"
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
                          <svg
                            class="w-5 h-5"
                            fill="none"
                            stroke="currentColor"
                            viewBox="0 0 24 24"
                          >
                            <path
                              stroke-linecap="round"
                              stroke-linejoin="round"
                              stroke-width="2"
                              d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
                            />
                          </svg>
                          <span>${this.tokenError}</span>
                        </div>
                      `
                    : ""}
                  ${this.loadingToken
                    ? html`
                        <div class="mt-4 text-center py-4">
                          <span
                            class="loading loading-spinner loading-md"
                          ></span>
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

        <!-- Nudges Modal -->
        <nudges-modal
          .open=${this.showNudgesModal}
          @close=${() => (this.showNudgesModal = false)}
          @nudges-saved=${() => (this.showNudgesModal = false)}
        ></nudges-modal>
      </div>
    `;
  }
}
