import { LitElement, html } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { api } from '../api';
import { tailwindStyles } from '../styles/shared';

@customElement('login-page')
export class LoginPage extends LitElement {
  static styles = [tailwindStyles];

  @property({ type: String }) error: string | null = null;
  @state() loading = false;
  @state() authError: string | null = null;

  async handleLogin() {
    this.loading = true;
    this.authError = null;
    try {
      const { url } = await api.getAuthUrl();
      window.location.href = url;
    } catch (e) {
      console.error('Failed to get auth URL:', e);
      this.authError = e instanceof Error ? e.message : 'Failed to start login';
      this.loading = false;
    }
  }

  render() {
    return html`
      <div class="hero min-h-screen">
        <div class="hero-content text-center">
          <div class="max-w-md">
            <h1 class="text-5xl font-bold">Cleo</h1>
            <p class="py-6">
              AI-powered social media content from your screen recordings.
              Review and post tweet-worthy moments with one click.
            </p>
            ${this.error || this.authError
              ? html`<div class="alert alert-error mb-4">
                  <svg xmlns="http://www.w3.org/2000/svg" class="stroke-current shrink-0 h-6 w-6" fill="none" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z" />
                  </svg>
                  <span>${this.error || this.authError}</span>
                </div>`
              : ''}
            <button
              class="btn btn-primary btn-lg"
              @click=${this.handleLogin}
              ?disabled=${this.loading}
            >
              ${this.loading
                ? html`<span class="loading loading-spinner"></span>`
                : html`
                    <svg class="w-6 h-6 mr-2" viewBox="0 0 24 24" fill="currentColor">
                      <path d="M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z"/>
                    </svg>
                    Sign in with X
                  `}
            </button>
          </div>
        </div>
      </div>
    `;
  }
}
