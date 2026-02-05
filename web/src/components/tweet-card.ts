import { LitElement, html } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import { ThreadTweet, api, PublishProgress } from "../api";
import { tailwindStyles } from "../styles/shared";
import "./card-shell";
import "./card-header";
import "./content-badge";
import "./tweet-content";

@customElement("tweet-card")
export class TweetCard extends LitElement {
  static styles = [tailwindStyles];

  @property({ type: Object }) tweet: ThreadTweet | null = null;
  @property({ type: Boolean }) hideActions = false;
  @property({ type: Boolean }) compact = false;
  @property({ type: Boolean }) showRailConnector = false;

  @state() posting = false;
  @state() dismissing = false;
  @state() uploadProgress: number | null = null;
  @state() uploadStatus: "uploading" | "processing" | "posting" | null = null;
  @state() error: string | null = null;
  @state() selectedCopyIndex = 0;
  @state() copyChoices: string[] = [];
  private lastTweetId: number | null = null;

  protected updated(changedProperties: Map<string, unknown>) {
    super.updated(changedProperties);
    if (changedProperties.has("tweet")) {
      const tweetId = this.tweet?.id ?? null;
      if (tweetId !== this.lastTweetId) {
        this.lastTweetId = tweetId;
        this.selectedCopyIndex = 0;
        this.copyChoices = this.tweet
          ? [this.tweet.text, ...this.tweet.copy_options]
          : [];
      }
    }
  }

  formatDate(dateStr: string) {
    return new Date(dateStr).toLocaleString();
  }

  async handlePost() {
    if (!this.tweet) return;

    this.posting = true;
    this.uploadProgress = null;
    this.uploadStatus = null;
    this.error = null;

    try {
      // Use WebSocket progress for media tweets (video or multiple images)
      const hasMedia =
        this.tweet.video_clip || this.tweet.image_capture_ids.length > 0;
      if (hasMedia) {
        await api.postTweetWithProgress(
          this.tweet.id,
          (progress: PublishProgress) => {
            if (progress.type === "uploading") {
              this.uploadStatus = "uploading";
              this.uploadProgress = progress.percent;
            } else if (progress.type === "processing") {
              this.uploadStatus = "processing";
              this.uploadProgress = null;
            } else if (progress.type === "posting") {
              this.uploadStatus = "posting";
            }
          },
        );
      } else {
        await api.postTweet(this.tweet.id);
      }
      this.dispatchEvent(
        new CustomEvent("tweet-posted", { detail: this.tweet.id }),
      );
    } catch (e) {
      console.error("Failed to post tweet:", e);
      this.error = e instanceof Error ? e.message : "Failed to post tweet";
    } finally {
      this.posting = false;
      this.uploadProgress = null;
      this.uploadStatus = null;
    }
  }

  async handleDismiss() {
    if (!this.tweet) return;

    this.dismissing = true;
    this.error = null;

    try {
      await api.dismissTweet(this.tweet.id);
      this.dispatchEvent(
        new CustomEvent("tweet-dismissed", { detail: this.tweet.id }),
      );
    } catch (e) {
      console.error("Failed to dismiss tweet:", e);
      this.error = e instanceof Error ? e.message : "Failed to dismiss tweet";
    } finally {
      this.dismissing = false;
    }
  }

  clearError() {
    this.error = null;
  }

  private async selectCopy(index: number) {
    if (!this.tweet) return;
    const newText = this.copyChoices[index];
    if (!newText) return;

    this.selectedCopyIndex = index;
    this.tweet = { ...this.tweet, text: newText };
    try {
      await api.updateTweetCollateral(this.tweet.id, { text: newText });
      this.dispatchEvent(
        new CustomEvent("collateral-updated", {
          detail: { text: newText },
          bubbles: true,
          composed: true,
        }),
      );
    } catch (e) {
      console.error("Failed to update tweet copy:", e);
      this.error =
        e instanceof Error ? e.message : "Failed to update tweet copy";
    }
  }

  handleCollateralUpdated(e: CustomEvent) {
    // Re-dispatch the event from tweet-content
    this.dispatchEvent(
      new CustomEvent("collateral-updated", {
        detail: e.detail,
        bubbles: true,
        composed: true,
      }),
    );
  }

  render() {
    if (!this.tweet) {
      return html`<card-shell
        ><p class="text-base-content/50">No tweet data</p></card-shell
      >`;
    }

    const imageCount = this.tweet.image_capture_ids.length;

    return html`
      <div class="relative ${this.copyChoices.length > 1 ? 'ml-6' : ''}">
        ${this.copyChoices.length > 1 ? html`
          <div class="absolute -left-6 top-3 flex flex-col gap-0.5">
            ${this.copyChoices.map((_, i) => html`
              <button
                class="w-6 h-7 text-[10px] font-semibold rounded-l-md border border-r-0 transition-colors
                  ${i === this.selectedCopyIndex
                    ? 'bg-primary text-primary-content border-primary'
                    : 'bg-base-200 text-base-content/60 border-base-300 hover:bg-base-300'}"
                @click=${() => this.selectCopy(i)}
              >${i === 0 ? 'A' : i === 1 ? 'B' : 'C'}</button>
            `)}
          </div>
        ` : ''}
        <card-shell
          ?showRailConnector=${this.showRailConnector}
          variant=${this.compact ? "compact" : "default"}
          class="max-w-2xl w-full max-h-sm"
        >
          <card-header slot="header">
            <span slot="left" class="text-xs">${this.formatDate(this.tweet.created_at)}</span>
            <div slot="right" class="flex gap-1.5">
            <slot name="extra-badges"></slot>
            ${this.tweet.video_clip
              ? html`<content-badge variant="accent">Video</content-badge>`
              : ""}
            ${imageCount > 0
              ? html`<content-badge variant="muted"
                  >${imageCount}
                  image${imageCount > 1 ? "s" : ""}</content-badge
                >`
              : ""}
          </div>
        </card-header>

        <tweet-content
          .tweet=${this.tweet}
          ?compact=${this.compact}
          @collateral-updated=${this.handleCollateralUpdated}
        ></tweet-content>

        ${this.error
          ? html`
              <div class="alert alert-error mt-2 py-1.5 px-2.5 text-xs">
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  class="stroke-current shrink-0 h-4 w-4"
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
                <span>${this.error}</span>
                <button class="btn btn-ghost btn-xs" @click=${this.clearError}>
                  Dismiss
                </button>
              </div>
            `
          : ""}
        ${!this.hideActions
          ? html`
              <div
                slot="actions"
                class="flex justify-end gap-2 mt-3 pt-2.5 border-t border-base-200"
              >
                <button
                  class="btn btn-ghost btn-sm"
                  @click=${this.handleDismiss}
                  ?disabled=${this.dismissing || this.posting}
                >
                  ${this.dismissing
                    ? html`<span
                        class="loading loading-spinner loading-sm"
                      ></span>`
                    : "Dismiss"}
                </button>
                <button
                  class="btn btn-primary btn-sm gap-1 min-w-24"
                  @click=${this.handlePost}
                  ?disabled=${this.posting || this.dismissing}
                >
                  ${this.posting
                    ? this.uploadStatus === "uploading" &&
                      this.uploadProgress !== null
                      ? html`
                          <div class="flex items-center gap-2">
                            <div
                              class="radial-progress text-primary-content"
                              style="--value:${this
                                .uploadProgress}; --size:1.25rem; --thickness:2px;"
                            ></div>
                            <span class="text-xs">${this.uploadProgress}%</span>
                          </div>
                        `
                      : this.uploadStatus === "processing"
                        ? html`<span
                              class="loading loading-spinner loading-sm"
                            ></span
                            ><span class="text-xs">Processing...</span>`
                        : this.uploadStatus === "posting"
                          ? html`<span
                                class="loading loading-spinner loading-sm"
                              ></span
                              ><span class="text-xs">Posting...</span>`
                          : html`<span
                              class="loading loading-spinner loading-sm"
                            ></span>`
                    : html`
                        <svg
                          class="w-4 h-4"
                          viewBox="0 0 24 24"
                          fill="currentColor"
                        >
                          <path
                            d="M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z"
                          />
                        </svg>
                        Post
                      `}
                </button>
              </div>
            `
          : ""}
        </card-shell>
      </div>
    `;
  }
}
