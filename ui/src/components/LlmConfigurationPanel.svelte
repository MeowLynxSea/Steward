<script lang="ts">
  import {
    Bot,
    BrainCircuit,
    Cloud,
    Code2,
    Cpu,
    KeyRound,
    Server,
    Sparkles,
    Plus,
    Pencil,
    Trash2,
    X
  } from "lucide-svelte";
  import { settingsStore } from "../lib/stores/settings.svelte";
  import { getOpenAiCodexLoginStatus, startOpenAiCodexLogin } from "../lib/tauri";
  import type { BackendInstance } from "../lib/types";

  type ProviderPreset = {
    id: string;
    label: string;
    description: string;
    icon: typeof Sparkles;
    defaultModel: string;
    supportsApiKey: boolean;
    supportsBaseUrl: boolean;
    defaultBaseUrl: string | null;
    baseUrlPlaceholder: string;
    supportsFormat: boolean;
    requiresOauth: boolean;
  };

  type Mode = "onboarding" | "settings";

  const providerPresets: ProviderPreset[] = [
    {
      id: "openai",
      label: "OpenAI",
      description: "Direct OpenAI API access with selectable Chat or Responses format.",
      icon: Sparkles,
      defaultModel: "gpt-5-mini",
      supportsApiKey: true,
      supportsBaseUrl: true,
      defaultBaseUrl: "https://api.openai.com/v1",
      baseUrlPlaceholder: "https://api.openai.com/v1",
      supportsFormat: true,
      requiresOauth: false
    },
    {
      id: "openai_codex",
      label: "Codex",
      description: "ChatGPT OAuth for the existing Codex workflow.",
      icon: Code2,
      defaultModel: "gpt-5.3-codex",
      supportsApiKey: false,
      supportsBaseUrl: false,
      defaultBaseUrl: null,
      baseUrlPlaceholder: "",
      supportsFormat: false,
      requiresOauth: true
    },
    {
      id: "anthropic",
      label: "Anthropic",
      description: "Claude via the Anthropic API.",
      icon: BrainCircuit,
      defaultModel: "claude-sonnet-4-20250514",
      supportsApiKey: true,
      supportsBaseUrl: true,
      defaultBaseUrl: "https://api.anthropic.com",
      baseUrlPlaceholder: "https://api.anthropic.com",
      supportsFormat: false,
      requiresOauth: false
    },
    {
      id: "groq",
      label: "Groq",
      description: "Low-latency hosted inference.",
      icon: Cpu,
      defaultModel: "llama-3.3-70b-versatile",
      supportsApiKey: true,
      supportsBaseUrl: false,
      defaultBaseUrl: null,
      baseUrlPlaceholder: "",
      supportsFormat: false,
      requiresOauth: false
    },
    {
      id: "openrouter",
      label: "OpenRouter",
      description: "Multi-provider routing with a single API key.",
      icon: Cloud,
      defaultModel: "openai/gpt-4o",
      supportsApiKey: true,
      supportsBaseUrl: false,
      defaultBaseUrl: null,
      baseUrlPlaceholder: "",
      supportsFormat: false,
      requiresOauth: false
    },
    {
      id: "ollama",
      label: "Ollama",
      description: "Local models running on this machine.",
      icon: Bot,
      defaultModel: "llama3",
      supportsApiKey: false,
      supportsBaseUrl: true,
      defaultBaseUrl: "http://127.0.0.1:11434",
      baseUrlPlaceholder: "http://127.0.0.1:11434",
      supportsFormat: false,
      requiresOauth: false
    }
  ];

  let {
    mode = "settings",
    title = "Model provider",
    eyebrow = "Settings",
    description = "Configure the model provider used by the desktop runtime.",
    submitLabel = "Save",
    onComplete
  }: {
    mode?: Mode;
    title?: string;
    eyebrow?: string;
    description?: string;
    submitLabel?: string;
    onComplete?: (() => void | Promise<void>) | undefined;
  } = $props();

  // Backend management state (for mode="settings")
  let showBackendForm = $state(false);
  let editingBackendId = $state<string | null>(null); // null = new backend
  let formProvider = $state("");
  let formApiKey = $state("");
  let formBaseUrl = $state("");
  let formModel = $state("");
  let formRequestFormat = $state("chat_completions");

  // Legacy state (for mode="onboarding")
  let codexLoginId = $state<string | null>(null);
  let codexVerificationUri = $state("");
  let codexUserCode = $state("");
  let codexLoginPending = $state(false);
  let codexLoginError = $state<string | null>(null);
  let codexPollTimer: number | null = null;

  const isSettingsMode = $derived(mode === "settings");
  const isOnboardingMode = $derived(mode === "onboarding");

  // Provider preset lookup
  function getProviderPreset(providerId: string): ProviderPreset | undefined {
    return providerPresets.find((p) => p.id === providerId);
  }

  // Backend form helpers
  function openAddBackend() {
    editingBackendId = null;
    formProvider = "openai";
    formApiKey = "";
    formBaseUrl = "";
    formModel = "";
    formRequestFormat = "chat_completions";
    showBackendForm = true;
  }

  function openEditBackend(backend: BackendInstance) {
    editingBackendId = backend.id;
    formProvider = backend.provider;
    formApiKey = backend.api_key ?? "";
    formBaseUrl = backend.base_url ?? "";
    formModel = backend.model;
    formRequestFormat = backend.request_format ?? "chat_completions";
    showBackendForm = true;
  }

  function closeBackendForm() {
    showBackendForm = false;
    editingBackendId = null;
  }

  function getSelectedProviderPreset(): ProviderPreset | undefined {
    return getProviderPreset(formProvider);
  }

  async function saveBackendForm() {
    const provider = getProviderPreset(formProvider);
    if (!provider || !formModel.trim()) {
      return;
    }

    const backendData: Partial<BackendInstance> = {
      provider: formProvider,
      model: formModel.trim(),
      api_key: formApiKey.trim() || null,
      base_url: formBaseUrl.trim() || null,
      request_format: provider.supportsFormat ? formRequestFormat : null
    };

    if (editingBackendId === null) {
      // New backend
      const newBackend: BackendInstance = {
        id: crypto.randomUUID(),
        ...backendData
      } as BackendInstance;
      settingsStore.addBackend(newBackend);
    } else {
      // Existing backend
      settingsStore.updateBackend(editingBackendId, backendData);
    }

    await settingsStore.save();
    closeBackendForm();
  }

  async function deleteBackend(id: string) {
    settingsStore.removeBackend(id);
    await settingsStore.save();
  }

  // Legacy functions (for onboarding mode)
  function normalizeBackendId(value: string | null) {
    return value?.trim().toLowerCase() ?? providerPresets[0].id;
  }

  const selectedProvider = $derived(
    providerPresets.find((provider) => provider.id === normalizeBackendId(settingsStore.data.llm_backend))
      ?? providerPresets[0]
  );

  const selectedOverride = $derived(
    settingsStore.data.llm_builtin_overrides[selectedProvider.id] ?? {
      api_key: null,
      model: null,
      base_url: null,
      request_format: null
    }
  );

  const currentBaseUrl = $derived.by(() => {
    if (selectedProvider.id === "ollama") {
      return settingsStore.data.ollama_base_url ?? "";
    }
    return selectedOverride.base_url ?? "";
  });

  const currentApiFormat = $derived(selectedOverride.request_format ?? "chat_completions");
  const cheapModelDisabled = $derived(settingsStore.data.cheap_model_uses_primary);

  function selectProvider(provider: ProviderPreset) {
    const isSameProvider = normalizeBackendId(settingsStore.data.llm_backend) === provider.id;
    const providerOverride = settingsStore.data.llm_builtin_overrides[provider.id];
    settingsStore.updateField("llm_backend", provider.id);
    settingsStore.updateField(
      "selected_model",
      isSameProvider ? (settingsStore.data.selected_model ?? provider.defaultModel) : provider.defaultModel
    );

    if (provider.supportsBaseUrl && provider.defaultBaseUrl) {
      if (provider.id === "ollama") {
        settingsStore.updateField("ollama_base_url", settingsStore.data.ollama_base_url ?? provider.defaultBaseUrl);
      } else {
        settingsStore.setBuiltinOverride(provider.id, {
          base_url: providerOverride?.base_url ?? provider.defaultBaseUrl
        });
      }
    }

    if (provider.id === "openai") {
      settingsStore.setBuiltinOverride("openai", {
        request_format: settingsStore.data.llm_builtin_overrides.openai?.request_format ?? "chat_completions"
      });
    }
  }

  function updateBaseUrl(value: string) {
    if (selectedProvider.id === "ollama") {
      settingsStore.updateField("ollama_base_url", value);
      return;
    }
    settingsStore.updateBuiltinOverride(selectedProvider.id, "base_url", value);
  }

  function stopCodexPolling() {
    if (codexPollTimer !== null) {
      window.clearInterval(codexPollTimer);
      codexPollTimer = null;
    }
  }

  async function pollCodexStatus() {
    if (!codexLoginId) {
      return;
    }

    try {
      const status = await getOpenAiCodexLoginStatus(codexLoginId);
      if (status.status === "success") {
        stopCodexPolling();
        codexLoginPending = false;
        codexLoginError = null;
        const saved = await settingsStore.save();
        if (!saved) {
          return;
        }
        if (onComplete && settingsStore.data.llm_ready) {
          await onComplete();
        }
        return;
      }

      if (status.status === "error") {
        stopCodexPolling();
        codexLoginPending = false;
        codexLoginError = status.message;
      }
    } catch (error) {
      stopCodexPolling();
      codexLoginPending = false;
      codexLoginError = error instanceof Error ? error.message : "Codex login failed";
    }
  }

  async function beginCodexLogin() {
    codexLoginError = null;
    codexLoginPending = true;
    try {
      const response = await startOpenAiCodexLogin();
      codexLoginId = response.login_id;
      codexVerificationUri = response.verification_uri;
      codexUserCode = response.user_code;
      stopCodexPolling();
      codexPollTimer = window.setInterval(() => {
        void pollCodexStatus();
      }, 2500);
      await pollCodexStatus();
    } catch (error) {
      codexLoginPending = false;
      codexLoginError = error instanceof Error ? error.message : "Failed to start Codex login";
    }
  }

  async function handleSubmit() {
    const saved = await settingsStore.save();
    if (saved && onComplete) {
      await onComplete();
    }
  }
</script>

<svelte:window onbeforeunload={stopCodexPolling} />

{#if isSettingsMode}
  <!-- Settings mode: Backend management UI -->
  <div class="backend-management">
    {#if showBackendForm}
      <!-- Backend form -->
      <div class="backend-form">
        <div class="form-header">
          <h4>{editingBackendId === null ? "添加 Backend" : "编辑 Backend"}</h4>
          <button class="icon-btn" type="button" onclick={closeBackendForm} aria-label="取消">
            <X size={18} strokeWidth={2} />
          </button>
        </div>

        <div class="form-body">
          <label class="field">
            <span>Provider</span>
            <select
              class="field-input"
              bind:value={formProvider}
            >
              {#each providerPresets as provider (provider.id)}
                <option value={provider.id}>{provider.label}</option>
              {/each}
            </select>
          </label>

          {#if getSelectedProviderPreset()?.supportsApiKey}
            <label class="field">
              <span>API Key</span>
              <div class="input-with-icon">
                <KeyRound size={16} strokeWidth={2} />
                <input
                  class="field-input"
                  type="password"
                  placeholder="Paste API key"
                  bind:value={formApiKey}
                />
              </div>
            </label>
          {/if}

          {#if getSelectedProviderPreset()?.supportsBaseUrl}
            <label class="field">
              <span>Base URL</span>
              <div class="input-with-icon">
                <Server size={16} strokeWidth={2} />
                <input
                  class="field-input"
                  type="text"
                  placeholder={getSelectedProviderPreset()?.baseUrlPlaceholder ?? ""}
                  bind:value={formBaseUrl}
                />
              </div>
            </label>
          {/if}

          <label class="field">
            <span>Model</span>
            <input
              class="field-input"
              type="text"
              placeholder={getSelectedProviderPreset()?.defaultModel ?? ""}
              bind:value={formModel}
            />
          </label>

          {#if getSelectedProviderPreset()?.supportsFormat}
            <label class="field">
              <span>Request Format</span>
              <div class="segmented-control">
                <button
                  class:active={formRequestFormat === "chat_completions"}
                  class="segment-button"
                  type="button"
                  onclick={() => formRequestFormat = "chat_completions"}
                >
                  Chat
                </button>
                <button
                  class:active={formRequestFormat === "responses"}
                  class="segment-button"
                  type="button"
                  onclick={() => formRequestFormat = "responses"}
                >
                  Responses
                </button>
              </div>
            </label>
          {/if}
        </div>

        <div class="form-actions">
          <button class="submit-button small" type="button" onclick={() => void saveBackendForm()}>
            保存
          </button>
        </div>
      </div>
    {:else}
      <!-- Backend list -->
      <div class="backend-list">
        {#if settingsStore.data.backends.length === 0}
          <div class="empty-state">
            <p>暂无配置的 Backend</p>
            <button class="add-backend-btn" type="button" onclick={openAddBackend}>
              <Plus size={16} strokeWidth={2} />
              <span>添加第一个 Backend</span>
            </button>
          </div>
        {:else}
          <div class="backend-grid">
            {#each settingsStore.data.backends as backend (backend.id)}
              {@const preset = getProviderPreset(backend.provider)}
              <div class="backend-card">
                <div class="backend-card-header">
                  {#if preset}
                    <preset.icon size={18} strokeWidth={2} />
                  {/if}
                  <span class="backend-provider">{preset?.label ?? backend.provider}</span>
                </div>
                <div class="backend-model">{backend.model}</div>
                <div class="backend-actions">
                  <button class="icon-btn" type="button" onclick={() => openEditBackend(backend)} aria-label="编辑">
                    <Pencil size={15} strokeWidth={2} />
                  </button>
                  <button class="icon-btn danger" type="button" onclick={() => deleteBackend(backend.id)} aria-label="删除">
                    <Trash2 size={15} strokeWidth={2} />
                  </button>
                </div>
              </div>
            {/each}

            <button class="add-backend-card" type="button" onclick={openAddBackend}>
              <Plus size={20} strokeWidth={2} />
              <span>添加 Backend</span>
            </button>
          </div>
        {/if}
      </div>
    {/if}
  </div>
{:else}
  <!-- Onboarding mode: Original provider selection UI -->
  <section class:fullscreen={mode === "onboarding"} class="configuration-shell">
    <div class:configuration-card-onboarding={mode === "onboarding"} class="configuration-card">
      <div class="configuration-scroll">
        <div class="hero-copy">
          <p class="eyebrow">{eyebrow}</p>
          <h2>{title}</h2>
          <p class="description">{description}</p>
        </div>

        <div class="provider-grid">
          {#each providerPresets as provider (provider.id)}
            <button
              class:selected={selectedProvider.id === provider.id}
              class="provider-card"
              type="button"
              onclick={() => selectProvider(provider)}
            >
              <provider.icon size={20} strokeWidth={2} />
              <div>
                <strong>{provider.label}</strong>
                <p>{provider.description}</p>
              </div>
            </button>
          {/each}
        </div>

        <div class="form-grid">
          <label class="field">
            <span>Model</span>
            <input
              value={settingsStore.data.selected_model ?? ""}
              placeholder={selectedProvider.defaultModel}
              oninput={(event) =>
                settingsStore.updateField("selected_model", (event.currentTarget as HTMLInputElement).value)}
            />
          </label>

          {#if selectedProvider.supportsFormat}
            <label class="field">
              <span>API Format</span>
              <div class="segmented-control">
                <button
                  class:active={currentApiFormat === "chat_completions"}
                  class="segment-button"
                  type="button"
                  onclick={() => settingsStore.updateBuiltinOverride("openai", "request_format", "chat_completions")}
                >
                  Chat
                </button>
                <button
                  class:active={currentApiFormat === "responses"}
                  class="segment-button"
                  type="button"
                  onclick={() => settingsStore.updateBuiltinOverride("openai", "request_format", "responses")}
                >
                  Responses
                </button>
              </div>
            </label>
          {/if}

          {#if selectedProvider.supportsApiKey}
            <label class="field">
              <span>API Key</span>
              <div class="input-with-icon">
                <KeyRound size={16} strokeWidth={2} />
                <input
                  value={selectedOverride.api_key ?? ""}
                  placeholder="Paste API key"
                  oninput={(event) =>
                    settingsStore.updateBuiltinOverride(
                      selectedProvider.id,
                      "api_key",
                      (event.currentTarget as HTMLInputElement).value
                    )}
                />
              </div>
            </label>
          {/if}

          {#if selectedProvider.supportsBaseUrl}
            <label class="field field-wide">
              <span>Base URL</span>
              <div class="input-with-icon">
                <Server size={16} strokeWidth={2} />
                <input
                  value={currentBaseUrl}
                  placeholder={selectedProvider.baseUrlPlaceholder}
                  oninput={(event) => updateBaseUrl((event.currentTarget as HTMLInputElement).value)}
                />
              </div>
            </label>
          {/if}
        </div>

        {#if selectedProvider.requiresOauth}
          <div class="oauth-panel">
            <div>
              <p class="oauth-title">ChatGPT OAuth</p>
              <p class="oauth-copy">Start the existing Codex OAuth flow directly inside the desktop app.</p>
            </div>

            <button class="submit-button secondary" type="button" onclick={() => void beginCodexLogin()}>
              {codexLoginPending ? "Waiting for authorization..." : "Start login"}
            </button>

            {#if codexVerificationUri && codexUserCode}
              <div class="oauth-card">
                <p>Open <strong>{codexVerificationUri}</strong> and enter this code:</p>
                <div class="oauth-code">{codexUserCode}</div>
              </div>
            {/if}

            {#if codexLoginError}
              <p class="status error">{codexLoginError}</p>
            {/if}
          </div>
        {/if}

        <div class="status-row">
          <div class="status-copy">
            {#if settingsStore.data.llm_ready}
              <p class="status ready">Provider ready</p>
            {:else if settingsStore.error}
              <p class="status error">{settingsStore.error}</p>
            {:else if settingsStore.data.llm_readiness_error}
              <p class="status pending">{settingsStore.data.llm_readiness_error}</p>
            {:else}
              <p class="status pending">Complete the provider setup to continue.</p>
            {/if}

            {#if settingsStore.status}
              <p class="subtle">{settingsStore.status}</p>
            {/if}
          </div>

          {#if !selectedProvider.requiresOauth}
            <button class="submit-button" type="button" onclick={() => void handleSubmit()}>
              {submitLabel}
            </button>
          {/if}
        </div>
      </div>
    </div>
  </section>
{/if}

<style>
  /* Backend management UI (settings mode) */
  .backend-management {
    width: 100%;
  }

  .backend-list {
    width: 100%;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding: 32px;
    text-align: center;
    color: var(--text-secondary);
    font-size: 14px;
  }

  .add-backend-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 10px 16px;
    border-radius: 14px;
    border: 1px dashed var(--border-default);
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .add-backend-btn:hover {
    border-color: var(--accent-primary);
    color: var(--accent-primary);
    background: color-mix(in srgb, var(--accent-primary) 8%, transparent);
    transform: translateY(-1px);
  }

  .backend-grid {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .backend-card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 16px;
    border-radius: 18px;
    border: 1px solid var(--border-default);
    background: var(--bg-surface);
    position: relative;
    transition: all 0.15s ease;
  }

  .backend-card:hover {
    transform: translateY(-1px);
    box-shadow: var(--shadow-card);
  }

  .backend-card-header {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--text-primary);
  }

  .backend-provider {
    font-size: 14px;
    font-weight: 600;
  }

  .backend-model {
    font-size: 13px;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .backend-actions {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }

  .add-backend-card {
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 16px;
    border-radius: 18px;
    border: 1px dashed var(--border-default);
    background: transparent;
    color: var(--text-tertiary);
    font: inherit;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .add-backend-card:hover {
    border-color: var(--accent-primary);
    color: var(--accent-primary);
    background: color-mix(in srgb, var(--accent-primary) 6%, transparent);
    transform: translateY(-1px);
  }

  .backend-form {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .form-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .form-header h4 {
    margin: 0;
    font-size: 16px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .form-body {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .form-actions {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    padding-top: 8px;
  }

  .submit-button.small {
    min-width: auto;
    min-height: 36px;
    padding: 0 16px;
    font-size: 13px;
    border-radius: 10px;
  }

  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border-radius: 10px;
    border: none;
    background: transparent;
    color: var(--text-tertiary);
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .icon-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
    transform: translateY(-1px);
  }

  .icon-btn.danger:hover {
    background: color-mix(in srgb, var(--accent-danger-text) 12%, transparent);
    color: var(--accent-danger-text);
  }

  .field-input {
    width: 100%;
    min-height: 42px;
    padding: 0 14px;
    border-radius: 12px;
    border: 1px solid var(--border-input);
    background: var(--bg-input);
    color: var(--text-primary);
    font: inherit;
    font-size: 13px;
    outline: none;
    transition: all 0.15s ease;
    box-sizing: border-box;
  }

  .field-input:focus {
    border-color: var(--accent-primary);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-primary) 15%, transparent);
  }

  .field-input:is(select) {
    appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%23888' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpolyline points='6 9 12 15 18 9'%3E%3C/polyline%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 14px center;
    padding-right: 38px;
    cursor: pointer;
  }

  .configuration-shell {
    width: 100%;
    min-height: 0;
  }

  .configuration-shell.fullscreen {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 0;
    height: 100%;
    max-height: 100%;
    overflow: hidden;
    padding: 16px;
    box-sizing: border-box;
    background: var(--bg-primary);
  }

  .configuration-card {
    width: min(960px, 100%);
    padding: 28px;
    border-radius: 28px;
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
    box-shadow: var(--shadow-container);
    backdrop-filter: blur(18px);
  }

  .configuration-card.configuration-card-onboarding {
    width: min(800px, 100%);
    max-width: 100%;
    height: auto;
    max-height: min(760px, calc(100% - 4px));
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 18px;
    border-radius: 22px;
    overflow: hidden;
    box-sizing: border-box;
  }

  .configuration-scroll {
    width: 100%;
  }

  .configuration-card-onboarding .configuration-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-right: 6px;
  }

  .hero-copy {
    margin-bottom: 24px;
  }

  .configuration-card-onboarding .hero-copy {
    margin-bottom: 16px;
  }

  .eyebrow {
    margin: 0 0 8px;
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  h2 {
    margin: 0;
    font-size: 32px;
    line-height: 1.05;
    color: var(--text-primary);
  }

  .configuration-card-onboarding h2 {
    font-size: 28px;
  }

  .description {
    margin: 10px 0 0;
    max-width: 620px;
    color: var(--text-secondary);
    font-size: 15px;
    line-height: 1.6;
  }

  .configuration-card-onboarding .description {
    margin-top: 8px;
    font-size: 14px;
    line-height: 1.5;
  }

  .provider-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: 12px;
    margin-bottom: 20px;
  }

  .configuration-card-onboarding .provider-grid {
    gap: 10px;
    margin-bottom: 16px;
  }

  .provider-card {
    display: flex;
    gap: 12px;
    align-items: flex-start;
    padding: 14px;
    border-radius: 18px;
    border: 1px solid var(--border-default);
    background: var(--bg-surface);
    color: var(--text-primary);
    text-align: left;
    cursor: pointer;
    transition: transform 0.18s ease, border-color 0.18s ease, box-shadow 0.18s ease;
  }

  .configuration-card-onboarding .provider-card {
    gap: 10px;
    padding: 12px;
    border-radius: 16px;
  }

  .provider-card.selected {
    border-color: var(--accent-gold);
    background: var(--bg-elevated);
    box-shadow: var(--shadow-card);
  }

  .provider-card strong {
    display: block;
    margin-bottom: 6px;
    font-size: 15px;
  }

  .provider-card p {
    margin: 0;
    color: var(--text-tertiary);
    font-size: 13px;
    line-height: 1.45;
  }

  .configuration-card-onboarding .provider-card p {
    font-size: 12px;
    line-height: 1.35;
  }

  .form-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 14px;
  }

  .configuration-card-onboarding .form-grid {
    gap: 12px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .field-wide {
    grid-column: 1 / -1;
  }

  .field span {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .field input {
    width: 100%;
    box-sizing: border-box;
    min-height: 48px;
    padding: 0 14px;
    border-radius: 14px;
    border: 1px solid var(--border-input);
    background: var(--bg-input);
    color: var(--text-primary);
    font: inherit;
    outline: none;
  }

  .segmented-control {
    display: inline-grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 6px;
    padding: 6px;
    border-radius: 16px;
    background: var(--bg-input);
    border: 1px solid var(--border-input);
  }

  .segment-button {
    min-height: 40px;
    padding: 0 14px;
    border: none;
    border-radius: 12px;
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.18s ease, color 0.18s ease, box-shadow 0.18s ease;
  }

  .segment-button.active {
    background: var(--accent-primary);
    color: var(--text-on-dark);
    box-shadow: var(--shadow-card);
  }

  .input-with-icon {
    position: relative;
  }

  .input-with-icon :global(svg) {
    position: absolute;
    top: 16px;
    left: 14px;
    color: var(--text-muted);
  }

  .input-with-icon input {
    padding-left: 40px;
  }

  .oauth-panel {
    margin-top: 18px;
    padding: 16px;
    border-radius: 18px;
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
  }

  .configuration-card-onboarding .oauth-panel {
    margin-top: 14px;
    padding: 14px;
    border-radius: 16px;
  }

  .oauth-title {
    margin: 0;
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .oauth-copy {
    margin: 6px 0 0;
    color: var(--text-secondary);
    font-size: 14px;
  }

  .oauth-card {
    margin-top: 16px;
    padding: 14px;
    border-radius: 16px;
    background: var(--bg-elevated);
  }

  .oauth-card p {
    margin: 0 0 10px;
  }

  .oauth-code {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-height: 48px;
    padding: 0 18px;
    border-radius: 14px;
    background: var(--accent-primary);
    color: var(--text-on-dark);
    font-size: 18px;
    font-weight: 700;
    letter-spacing: 0.16em;
  }

  .status-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    margin-top: 20px;
  }

  .configuration-card-onboarding .status-row {
    margin-top: 14px;
  }

  .status-copy {
    min-height: 40px;
  }

  .status,
  .subtle {
    margin: 0;
  }

  .status {
    font-size: 14px;
    font-weight: 600;
  }

  .status.ready {
    color: var(--accent-green);
  }

  .status.pending {
    color: var(--accent-gold);
  }

  .status.error {
    color: var(--accent-danger-text);
  }

  .subtle {
    margin-top: 4px;
    font-size: 13px;
    color: var(--text-tertiary);
  }

  .submit-button {
    min-width: 160px;
    min-height: 48px;
    padding: 0 18px;
    border: none;
    border-radius: 14px;
    background: var(--accent-primary);
    color: var(--text-on-dark);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  .submit-button.secondary {
    margin-top: 16px;
    background: var(--bg-elevated);
    color: var(--text-primary);
  }

  @media (max-width: 720px) {
    .configuration-shell.fullscreen {
      padding: 12px;
    }

    .configuration-card {
      padding: 18px;
      border-radius: 22px;
    }

    .form-grid {
      grid-template-columns: 1fr;
    }

    .status-row {
      flex-direction: column;
      align-items: stretch;
    }

    .submit-button {
      width: 100%;
    }
  }
</style>
