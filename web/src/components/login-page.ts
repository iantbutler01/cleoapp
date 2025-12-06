import { LitElement, html, css } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { api } from '../api';

@customElement('login-page')
export class LoginPage extends LitElement {
  @state() loading = false;

  // Use Tailwind/DaisyUI classes directly
  createRenderRoot() {
    return this;
  }

  async handleLogin() {
    this.loading = true;
    try {
      const { url } = await api.getAuthUrl();
      window.location.href = url;
    } catch (e) {
      console.error('Failed to get auth URL:', e);
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
