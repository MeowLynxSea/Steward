import { apiClient } from "../api";
import type { SettingsResponse } from "../types";

const DEFAULT_SETTINGS: SettingsResponse = {
  llm_backend: null,
  selected_model: null,
  ollama_base_url: null,
  openai_compatible_base_url: null,
  llm_custom_providers: [],
  llm_builtin_overrides: {}
};

class SettingsState {
  data = $state<SettingsResponse>(structuredClone(DEFAULT_SETTINGS));
  loading = $state(false);
  error = $state<string | null>(null);
  status = $state<string>("");

  async fetch() {
    this.loading = true;
    this.error = null;
    try {
      this.data = await apiClient.getSettings();
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to load settings";
    } finally {
      this.loading = false;
    }
  }

  updateField<K extends keyof SettingsResponse>(key: K, value: string) {
    this.data = { ...this.data, [key]: value || null };
  }

  async save() {
    this.error = null;
    try {
      this.data = await apiClient.patchSettings({
        llm_backend: this.data.llm_backend,
        selected_model: this.data.selected_model,
        ollama_base_url: this.data.ollama_base_url,
        openai_compatible_base_url: this.data.openai_compatible_base_url,
        llm_builtin_overrides: this.data.llm_builtin_overrides
      });
      this.status = "Settings saved";
      return true;
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to save settings";
      return false;
    }
  }
}

export const settingsStore = new SettingsState();
