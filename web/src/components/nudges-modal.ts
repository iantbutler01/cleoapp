import { LitElement, html } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import { api, Persona, UserPersona } from "../api";
import { tailwindStyles } from "../styles/shared";

@customElement("nudges-modal")
export class NudgesModal extends LitElement {
  static styles = [tailwindStyles];

  @property({ type: Boolean }) open = false;

  @state() private nudges = "";
  @state() private selectedPersonaId: number | null = null;
  @state() private systemPersonas: Persona[] = [];
  @state() private userPersonas: UserPersona[] = [];
  @state() private loading = true;
  @state() private saving = false;
  @state() private error: string | null = null;
  @state() private showSaveAsDialog = false;
  @state() private newPersonaName = "";

  async connectedCallback() {
    super.connectedCallback();
    document.addEventListener("keydown", this.handleKeydown);
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    document.removeEventListener("keydown", this.handleKeydown);
  }

  private handleKeydown = (e: KeyboardEvent) => {
    if (e.key === "Escape" && this.open) {
      this.close();
    }
  };

  protected updated(changedProperties: Map<string, unknown>) {
    if (changedProperties.has("open") && this.open) {
      this.loadData();
    }
  }

  private async loadData() {
    this.loading = true;
    this.error = null;

    try {
      const [personas, userPersonas, nudgesResp] = await Promise.all([
        api.getPersonas(),
        api.getUserPersonas(),
        api.getNudges(),
      ]);

      this.systemPersonas = personas;
      this.userPersonas = userPersonas;
      this.nudges = nudgesResp.nudges ?? "";
      this.selectedPersonaId = nudgesResp.selected_persona_id;
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to load";
    } finally {
      this.loading = false;
    }
  }

  private selectPersona(persona: Persona | UserPersona, isSystem: boolean) {
    this.nudges = persona.nudges;
    this.selectedPersonaId = isSystem ? persona.id : null;
  }

  private clearPersona() {
    this.selectedPersonaId = null;
  }

  private async save() {
    this.saving = true;
    this.error = null;

    try {
      await api.updateNudges(this.nudges, this.selectedPersonaId);
      this.dispatchEvent(new CustomEvent("nudges-saved", { bubbles: true }));
      this.close();
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to save";
    } finally {
      this.saving = false;
    }
  }

  private openSaveAsDialog() {
    this.showSaveAsDialog = true;
    this.newPersonaName = "";
  }

  private closeSaveAsDialog() {
    this.showSaveAsDialog = false;
    this.newPersonaName = "";
  }

  private async saveAsPersona() {
    if (!this.newPersonaName.trim()) return;

    this.saving = true;
    this.error = null;

    try {
      // First save the nudges
      await api.updateNudges(this.nudges, null);
      // Then create the persona from current nudges
      const newPersona = await api.createUserPersona(this.newPersonaName.trim());
      this.userPersonas = [...this.userPersonas, newPersona];
      this.closeSaveAsDialog();
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to save persona";
    } finally {
      this.saving = false;
    }
  }

  private async deleteUserPersona(id: number) {
    try {
      await api.deleteUserPersona(id);
      this.userPersonas = this.userPersonas.filter((p) => p.id !== id);
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to delete";
    }
  }

  private close() {
    this.showSaveAsDialog = false;
    this.newPersonaName = "";
    this.dispatchEvent(new CustomEvent("close", { bubbles: true }));
  }

  render() {
    if (!this.open) return html``;

    return html`
      <div class="modal modal-open">
        <div class="modal-box max-w-2xl p-0 overflow-hidden">
          <!-- Header -->
          <div class="px-6 py-4 border-b border-base-300/50">
            <h3 class="font-semibold text-base">Voice & Style</h3>
            <p class="text-xs text-base-content/50 mt-0.5">
              Customize how your tweets sound
            </p>
          </div>

          ${this.loading
            ? html`
                <div class="flex justify-center py-12">
                  <span class="loading loading-spinner loading-md"></span>
                </div>
              `
            : html`
                <div class="px-6 py-5">
                  ${this.error
                    ? html`
                        <div class="alert alert-error mb-4 text-sm py-2">
                          <span>${this.error}</span>
                        </div>
                      `
                    : ""}

                  <!-- Persona chips -->
                  <div class="mb-5">
                    <div class="text-xs font-medium text-base-content/60 mb-2">
                      Templates
                    </div>
                    <div class="flex flex-wrap gap-1.5">
                      ${this.systemPersonas.map(
                        (p) => html`
                          <button
                            class="px-3 py-1.5 text-xs rounded-full transition-colors ${this
                              .selectedPersonaId === p.id
                              ? "bg-primary text-primary-content"
                              : "bg-base-200 hover:bg-base-300 text-base-content"}"
                            @click=${() => this.selectPersona(p, true)}
                          >
                            ${p.name}
                          </button>
                        `
                      )}
                      ${this.userPersonas.map(
                        (p) => html`
                          <div class="inline-flex items-center gap-0.5">
                            <button
                              class="px-3 py-1.5 text-xs rounded-l-full bg-base-200 hover:bg-base-300 text-base-content transition-colors"
                              @click=${() => this.selectPersona(p, false)}
                            >
                              ${p.name}
                            </button>
                            <button
                              class="px-1.5 py-1.5 text-xs rounded-r-full bg-base-200 hover:bg-error/20 hover:text-error text-base-content/50 transition-colors"
                              @click=${() => this.deleteUserPersona(p.id)}
                              title="Delete"
                            >
                              <svg
                                class="w-3 h-3"
                                fill="none"
                                stroke="currentColor"
                                viewBox="0 0 24 24"
                              >
                                <path
                                  stroke-linecap="round"
                                  stroke-linejoin="round"
                                  stroke-width="2"
                                  d="M6 18L18 6M6 6l12 12"
                                />
                              </svg>
                            </button>
                          </div>
                        `
                      )}
                    </div>
                  </div>

                  <!-- Nudges textarea -->
                  <div>
                    <div class="text-xs font-medium text-base-content/60 mb-2">
                      Your voice
                    </div>
                    <textarea
                      class="w-full h-40 px-3 py-2.5 text-sm bg-base-200/50 border border-base-300/50 rounded-lg resize-none focus:outline-none focus:border-primary/50 focus:bg-base-100 transition-colors"
                      placeholder="Describe how you write - your tone, topics, what to avoid..."
                      .value=${this.nudges}
                      @input=${(e: Event) => {
                        this.nudges = (e.target as HTMLTextAreaElement).value;
                        this.clearPersona();
                      }}
                    ></textarea>
                    <div class="text-[10px] text-base-content/40 mt-1 text-right">
                      ${this.nudges.length} / 2000
                    </div>
                  </div>

                  <!-- Save as persona inline -->
                  ${this.showSaveAsDialog
                    ? html`
                        <div class="mt-4 pt-3 border-t border-base-300/30">
                          <div class="flex gap-2">
                            <input
                              type="text"
                              class="flex-1 px-3 py-2 text-sm bg-base-100 border border-base-300/50 rounded-lg focus:outline-none focus:border-primary/50"
                              placeholder="Name this persona..."
                              .value=${this.newPersonaName}
                              @input=${(e: Event) => {
                                this.newPersonaName = (
                                  e.target as HTMLInputElement
                                ).value;
                              }}
                              @keydown=${(e: KeyboardEvent) => {
                                if (e.key === "Enter") this.saveAsPersona();
                                if (e.key === "Escape")
                                  this.closeSaveAsDialog();
                              }}
                            />
                            <button
                              class="px-4 py-2 text-sm bg-primary text-primary-content rounded-lg hover:bg-primary/90 disabled:opacity-50 transition-colors"
                              @click=${this.saveAsPersona}
                              ?disabled=${!this.newPersonaName.trim() ||
                              this.saving}
                            >
                              Save
                            </button>
                            <button
                              class="px-3 py-2 text-sm text-base-content/70 hover:text-base-content transition-colors"
                              @click=${this.closeSaveAsDialog}
                            >
                              Cancel
                            </button>
                          </div>
                        </div>
                      `
                    : ""}
                </div>
              `}

          <!-- Footer -->
          <div
            class="px-6 py-3 border-t border-base-300/50 flex justify-between items-center bg-base-200/30"
          >
            <div>
              ${!this.loading && this.nudges.trim() && !this.showSaveAsDialog
                ? html`
                    <button
                      class="text-xs text-base-content/60 hover:text-base-content transition-colors"
                      @click=${this.openSaveAsDialog}
                    >
                      Save as template...
                    </button>
                  `
                : ""}
            </div>
            <div class="flex gap-2">
              <button
                class="px-4 py-2 text-sm text-base-content/70 hover:text-base-content transition-colors"
                @click=${this.close}
              >
                Cancel
              </button>
              <button
                class="px-4 py-2 text-sm bg-primary text-primary-content rounded-lg hover:bg-primary/90 disabled:opacity-50 transition-colors"
                @click=${this.save}
                ?disabled=${this.loading || this.saving}
              >
                ${this.saving
                  ? html`<span
                      class="loading loading-spinner loading-xs"
                    ></span>`
                  : "Save"}
              </button>
            </div>
          </div>
        </div>
        <div class="modal-backdrop bg-black/50" @click=${this.close}></div>
      </div>
    `;
  }
}
