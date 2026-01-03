import { LitElement, html } from 'lit';
import { customElement, state } from 'lit/decorators.js';
import { api } from '../api';
import { tailwindStyles } from '../styles/shared';
import './login-page';
import './dashboard-page';

@customElement('cleo-app')
export class CleoApp extends LitElement {
  static styles = [tailwindStyles];

  @state() isLoggedIn = false;
  @state() loading = true;
  @state() authError: string | null = null;

  async connectedCallback() {
    super.connectedCallback();

    // Set up callback for when session expires
    api.setUnauthorizedCallback(() => {
      this.isLoggedIn = false;
    });

    await this.checkAuth();
  }

  async checkAuth() {
    // Check if we're handling OAuth callback
    const params = new URLSearchParams(window.location.search);
    const code = params.get('code');
    const stateParam = params.get('state');

    if (code && stateParam) {
      // OAuth callback - exchange code for session (sets httpOnly cookies)
      try {
        await api.exchangeToken(code, stateParam);
        this.isLoggedIn = true;
        this.authError = null;
      } catch (e) {
        console.error('OAuth callback failed:', e);
        this.authError = 'Login failed. Please try again.';
      }
      // Clean up URL
      window.history.replaceState({}, '', '/');
      this.loading = false;
      return;
    }

    // Check if we have a valid session by calling /auth/me
    try {
      await api.checkSession();
      this.isLoggedIn = true;
    } catch {
      this.isLoggedIn = false;
    }

    this.loading = false;
  }

  async handleLogout() {
    await api.logout();
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
      : html`<login-page .error=${this.authError}></login-page>`;
  }
}
