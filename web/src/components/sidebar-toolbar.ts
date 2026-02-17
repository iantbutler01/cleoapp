import { LitElement, html } from "lit";
import { customElement, state } from "lit/decorators.js";
import { tailwindStyles } from "../styles/shared";
import { api } from "../api";

@customElement("sidebar-toolbar")
export class SidebarToolbar extends LitElement {
  static styles = [tailwindStyles];

  @state() private agentRunning = false;

  private openNudges() {
    this.dispatchEvent(
      new CustomEvent("open-nudges", { bubbles: true, composed: true })
    );
  }

  private async runAgent() {
    if (this.agentRunning) return;
    this.agentRunning = true;
    try {
      const res = await api.triggerAgentRun();
      if (res.status === "already_running") {
        // Already running, poll until done
      } else {
        this.dispatchEvent(
          new CustomEvent("agent-run-started", { bubbles: true, composed: true })
        );
      }
      this.pollAgentStatus();
    } catch (e) {
      console.error("Failed to trigger agent run:", e);
      this.agentRunning = false;
    }
  }

  private async pollAgentStatus() {
    const poll = async () => {
      try {
        const { running } = await api.getAgentStatus();
        this.agentRunning = running;
        if (running) {
          setTimeout(poll, 3000);
        }
      } catch {
        this.agentRunning = false;
      }
    };
    setTimeout(poll, 3000);
  }

  render() {
    return html`
      <div
        class="sticky top-24 mt-2 flex flex-col gap-1 bg-base-100/90 backdrop-blur-sm rounded-r-lg py-1.5 px-2 border border-l-0 border-base-300/30 shadow-sm -ml-px"
      >
        <button
          class="w-6 h-6 flex items-center justify-center rounded text-base-content/50 hover:text-base-content hover:bg-base-200/50 transition-colors tooltip tooltip-right"
          data-tip="Voice & Style (⌘+.)"
          @click=${this.openNudges}
        >
          <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
              d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z" />
          </svg>
        </button>
        <button
          class="w-6 h-6 flex items-center justify-center rounded transition-colors tooltip tooltip-right ${this.agentRunning
            ? "text-primary animate-spin cursor-not-allowed"
            : "text-base-content/50 hover:text-base-content hover:bg-base-200/50"}"
          data-tip="${this.agentRunning ? "Agent running..." : "Run Agent"}"
          @click=${this.runAgent}
          ?disabled=${this.agentRunning}
        >
          <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
              d="${this.agentRunning
                ? "M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
                : "M14.752 11.168l-3.197-2.132A1 1 0 0010 9.87v4.263a1 1 0 001.555.832l3.197-2.132a1 1 0 000-1.664z M21 12a9 9 0 11-18 0 9 9 0 0118 0z"}" />
          </svg>
        </button>
      </div>
    `;
  }
}
