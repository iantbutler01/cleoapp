import { LitElement, html } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { api } from '../api';
import './login-page';
import './dashboard-page';

@customElement('cleo-app')
export class CleoApp extends LitElement {
  @state() isLoggedIn = false;
  @state() loading = true;

  createRenderRoot() {
    return this;
  }

  async connectedCallback() {
    super.connectedCallback();
    await this.checkAuth();
  }

  async checkAuth() {
    // Check if we're handling OAuth callback
    const params = new URLSearchParams(window.location.search);
    const code = params.get('code');
    const state = params.get('state');

    if (code && state) {
      // OAuth callback - exchange code for session
      try {
        const res = await fetch('/api/auth/twitter/token', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ code, state }),
        });
        if (res.ok) {
          const data = await res.json();
          api.setUserId(data.user_id);
          this.isLoggedIn = true;
        }
        // Clean up URL
        window.history.replaceState({}, '', '/');
      } catch (e) {
        console.error('OAuth callback failed:', e);
      }
    }

    // Check if we have a stored user_id
    if (api.getUserId()) {
      try {
        await api.getMe();
        this.isLoggedIn = true;
      } catch {
        api.clearUserId();
        this.isLoggedIn = false;
      }
    }

    this.loading = false;
  }

  handleLogout() {
    this.isLoggedIn = false;
  }

  render() {
    if (this.loading) {
      return html`
        <div class="flex justify-center items-center min-h-screen">
          <span class="loading loading-spinner loading-lg"></span>
        </div>
      `;
    }

    return this.isLoggedIn
      ? html`<dashboard-page @logout=${this.handleLogout}></dashboard-page>`
      : html`<login-page></login-page>`;
  }
}
