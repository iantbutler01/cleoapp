import { LitElement, html } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { ThreadWithTweets, api } from '../api';
import { tailwindStyles } from '../styles/shared';
import './card-shell';
import './card-header';
import './content-badge';
import './tweet-content';

@customElement('thread-card')
export class ThreadCard extends LitElement {
  static styles = [tailwindStyles];

  @property({ type: Object }) thread: ThreadWithTweets | null = null;
  @state() posting = false;
  @state() deleting = false;
  @state() error: string | null = null;

  formatDate(dateStr: string) {
    return new Date(dateStr).toLocaleString();
  }

  async handlePost() {
    if (!this.thread) return;

    this.posting = true;
    this.error = null;

    try {
      await api.postThread(this.thread.thread.id);
      this.dispatchEvent(new CustomEvent('thread-posted', { detail: this.thread.thread.id }));
    } catch (e) {
      console.error('Failed to post thread:', e);
      this.error = e instanceof Error ? e.message : 'Failed to post thread';
    } finally {
      this.posting = false;
    }
  }

  async handleDelete() {
    if (!this.thread) return;

    this.deleting = true;
    this.error = null;

    try {
      await api.deleteThread(this.thread.thread.id);
      this.dispatchEvent(new CustomEvent('thread-deleted', { detail: this.thread.thread.id }));
    } catch (e) {
      console.error('Failed to delete thread:', e);
      this.error = e instanceof Error ? e.message : 'Failed to delete thread';
    } finally {
      this.deleting = false;
    }
  }

  clearError() {
    this.error = null;
  }

  handleTweetUpdated() {
    if (!this.thread) return;
    this.dispatchEvent(new CustomEvent('thread-updated', { detail: this.thread.thread.id }));
  }

  render() {
    if (!this.thread) {
      return html`<card-shell><p class="text-base-content/50">No thread data</p></card-shell>`;
    }

    const { thread, tweets } = this.thread;
    const isPosted = thread.status === 'posted';
    const isPartialFailed = thread.status === 'partial_failed';

    return html`
      <card-shell>
        <!-- Header with thread info -->
        <card-header slot="header">
          <span slot="left" class="text-xs">${this.formatDate(thread.created_at)}</span>
          <div slot="right" class="flex items-center gap-1.5">
            <content-badge variant="accent">Thread</content-badge>
            <content-badge variant="muted">${tweets.length} tweets</content-badge>
            ${isPosted ? html`<content-badge variant="status-success">Posted</content-badge>` : ''}
            ${isPartialFailed ? html`<content-badge variant="status-warning">Partial</content-badge>` : ''}
          </div>
        </card-header>

        <!-- All tweets in one container with dividers -->
        ${tweets.map((tweet, i) => html`
          ${i > 0 ? html`
            <div class="border-t border-base-300/30 my-3 flex items-center gap-2">
              <span class="bg-primary text-primary-content text-xs w-5 h-5 rounded-full flex items-center justify-center font-bold -ml-1">
                ${i + 1}
              </span>
            </div>
          ` : ''}
          <tweet-content
            .tweet=${tweet}
            ?compact=${i > 0}
            ?showRationale=${i === 0}
            @collateral-updated=${this.handleTweetUpdated}
          ></tweet-content>
        `)}

        ${this.error ? html`
          <div class="alert alert-error mt-3 py-2 px-3 text-sm">
            <svg xmlns="http://www.w3.org/2000/svg" class="stroke-current shrink-0 h-5 w-5" fill="none" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
            <span>${this.error}</span>
            <button class="btn btn-ghost btn-xs" @click=${this.clearError}>Dismiss</button>
          </div>
        ` : ''}

        <!-- Actions -->
        ${!isPosted ? html`
          <div slot="actions" class="flex justify-end gap-2 mt-3 pt-2 border-t border-base-300/20">
            <button
              class="btn btn-ghost btn-sm"
              @click=${this.handleDelete}
              ?disabled=${this.deleting || this.posting}
            >
              ${this.deleting
                ? html`<span class="loading loading-spinner loading-sm"></span>`
                : 'Delete'}
            </button>
            <button
              class="btn btn-primary btn-sm"
              @click=${this.handlePost}
              ?disabled=${this.posting || this.deleting}
            >
              ${this.posting
                ? html`<span class="loading loading-spinner loading-sm"></span>`
                : html`
                    <svg class="w-4 h-4 mr-1" viewBox="0 0 24 24" fill="currentColor">
                      <path d="M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z"/>
                    </svg>
                    Post Thread (${tweets.length})
                  `}
            </button>
          </div>
        ` : ''}
      </card-shell>
    `;
  }
}
