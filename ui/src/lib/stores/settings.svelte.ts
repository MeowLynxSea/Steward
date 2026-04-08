import { apiClient } from "../api";
import type { BackendInstance, PatchSettingsRequest, SettingsResponse } from "../types";

const DEFAULT_SETTINGS: SettingsResponse = {
  backends: [],
  major_backend_id: null,
  cheap_backend_id: null,
  cheap_model_uses_primary: true,
  llm_ready: false,
  llm_onboarding_required: true,
  llm_readiness_error: null
};

function normalizeSettingsResponse(value: Partial<SettingsResponse> | null | undefined): SettingsResponse {
  return {
    ...structuredClone(DEFAULT_SETTINGS),
    ...value,
    backends: Array.isArray(value?.backends) ? value!.backends : []
  };
}

class SettingsState {
  data = $state<SettingsResponse>(structuredClone(DEFAULT_SETTINGS));
  loading = $state(false);
  error = $state<string | null>(null);
  status = $state<string>("");

  async fetch() {
    this.loading = true;
    this.error = null;
    try {
      this.data = normalizeSettingsResponse(await apiClient.getSettings());
      this.status = "";
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to load settings";
    } finally {
      this.loading = false;
    }
  }

  setCheapModelUsesPrimary(value: boolean) {
    this.data = { ...this.data, cheap_model_uses_primary: value };
  }

  // Backend management
  addBackend(backend: BackendInstance) {
    this.data = { ...this.data, backends: [...this.data.backends, backend] };
  }

  updateBackend(id: string, patch: Partial<BackendInstance>) {
    this.data = {
      ...this.data,
      backends: this.data.backends.map((b) => (b.id === id ? { ...b, ...patch } : b))
    };
  }

  removeBackend(id: string) {
    this.data = { ...this.data, backends: this.data.backends.filter((b) => b.id !== id) };
  }

  setMajorBackend(id: string | null) {
    this.data = { ...this.data, major_backend_id: id };
  }

  setCheapBackend(id: string | null) {
    this.data = { ...this.data, cheap_backend_id: id };
  }

  async save() {
    this.error = null;
    this.status = "";
    const payload: PatchSettingsRequest = {
      backends: this.data.backends,
      major_backend_id: this.data.major_backend_id,
      cheap_backend_id: this.data.cheap_backend_id,
      cheap_model_uses_primary: this.data.cheap_model_uses_primary
    };

    try {
      this.data = normalizeSettingsResponse(await apiClient.patchSettings(payload));
      this.status = this.data.llm_ready ? "Provider ready" : "Settings saved";
      return true;
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to save settings";
      return false;
    }
  }
}

export const settingsStore = new SettingsState();
