import { apiClient } from "../api";
import type { LlmBuiltinOverride, PatchSettingsRequest, SettingsResponse } from "../types";

const DEFAULT_SETTINGS: SettingsResponse = {
  llm_backend: null,
  selected_model: null,
  cheap_model: null,
  cheap_model_uses_primary: true,
  ollama_base_url: null,
  openai_compatible_base_url: null,
  llm_custom_providers: [],
  llm_builtin_overrides: {},
  llm_ready: false,
  llm_onboarding_required: true,
  llm_readiness_error: null
};

function normalizeText(value: string) {
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
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
      this.data = await apiClient.getSettings();
      this.status = "";
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to load settings";
    } finally {
      this.loading = false;
    }
  }

  updateField<K extends keyof SettingsResponse>(key: K, value: string) {
    this.data = { ...this.data, [key]: normalizeText(value) as SettingsResponse[K] };
  }

  setCheapModelUsesPrimary(value: boolean) {
    this.data = { ...this.data, cheap_model_uses_primary: value };
  }

  setBuiltinOverride(providerId: string, patch: Partial<LlmBuiltinOverride>) {
    const current = this.data.llm_builtin_overrides[providerId] ?? {
      api_key: null,
      model: null,
      base_url: null,
      request_format: null
    };

    this.data = {
      ...this.data,
      llm_builtin_overrides: {
        ...this.data.llm_builtin_overrides,
        [providerId]: {
          ...current,
          ...patch
        }
      }
    };
  }

  updateBuiltinOverride(providerId: string, key: keyof LlmBuiltinOverride, value: string) {
    this.setBuiltinOverride(providerId, {
      [key]: normalizeText(value)
    });
  }

  async save() {
    this.error = null;
    this.status = "";
    const payload: PatchSettingsRequest = {
      llm_backend: this.data.llm_backend,
      selected_model: this.data.selected_model,
      cheap_model: this.data.cheap_model,
      cheap_model_uses_primary: this.data.cheap_model_uses_primary,
      ollama_base_url: this.data.ollama_base_url,
      openai_compatible_base_url: this.data.openai_compatible_base_url,
      llm_custom_providers: this.data.llm_custom_providers,
      llm_builtin_overrides: this.data.llm_builtin_overrides
    };

    try {
      this.data = await apiClient.patchSettings(payload);
      this.status = this.data.llm_ready ? "Provider ready" : "Settings saved";
      return true;
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to save settings";
      return false;
    }
  }
}

export const settingsStore = new SettingsState();
