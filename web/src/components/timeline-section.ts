import { LitElement, html } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { PendingTweet } from '../api';
import { formatRelativeTime, formatTimeOfDay } from '../utils/time-grouping';
import { tailwindStyles } from '../styles/shared';
import './tweet-card';

@customElement('timeline-section')
export class TimelineSection extends LitElement {
  static styles = [tailwindStyles];

  @property({ type: String }) label = '';
  @property({ type: Array }) tweets: PendingTweet[] = [];
  @property({ type: Boolean }) initiallyCollapsed = false;

  @state() collapsed = false;

  connectedCallback() {
    super.connectedCallback();
    this.collapsed = this.initiallyCollapsed;
  }

  toggleCollapse() {
    this.collapsed = !this.collapsed;
  }

  handleTweetAction(e: CustomEvent) {
    // Re-dispatch the event to bubble up
    this.dispatchEvent(new CustomEvent(e.type, { detail: e.detail, bubbles: true }));
  }

  render() {
    const tweetCount = this.tweets.length;

    return html`
      <div class="mb-6">
        <!-- Section Header -->
        <button
          class="flex items-center gap-2 w-full text-left py-2 px-1 hover:bg-base-200 rounded-lg transition-colors group"
          @click=${this.toggleCollapse}
        >
          <svg
            class="w-4 h-4 transition-transform ${this.collapsed ? '' : 'rotate-90'}"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7" />
          </svg>
          <span class="font-semibold text-lg">${this.label}</span>
          <span class="badge badge-ghost badge-sm">${tweetCount}</span>
        </button>

        <!-- Timeline Content -->
        ${!this.collapsed
          ? html`
              <div class="relative ml-3 pl-6 border-l-2 border-base-300 mt-2">
                ${this.tweets.map(
                  (tweet) => html`
                    <div class="relative mb-6 last:mb-0">
                      <!-- Timeline dot: center 12px dot on border at pl-6 edge -->
                      <div
                        class="absolute -left-[31px] top-1 w-3 h-3 rounded-full bg-primary border-2 border-base-100"
                      ></div>

                      <!-- Time label -->
                      <div class="flex items-center gap-2 mb-3 text-sm opacity-60">
                        <span>${formatTimeOfDay(tweet.created_at)}</span>
                        <span class="text-xs">${formatRelativeTime(tweet.created_at)}</span>
                      </div>

                      <!-- Tweet card -->
                      <tweet-card
                        .tweet=${tweet}
                        @tweet-posted=${this.handleTweetAction}
                        @tweet-dismissed=${this.handleTweetAction}
                      ></tweet-card>
                    </div>
                  `
                )}
              </div>
            `
          : html`
              <div class="ml-6 mt-2 text-sm opacity-50">
                ${tweetCount} tweet${tweetCount !== 1 ? 's' : ''} hidden
              </div>
            `}
      </div>
    `;
  }
}
