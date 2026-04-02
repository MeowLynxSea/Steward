<script lang="ts">
  import {
    Bot,
    BrainCircuit,
    Cloud,
    Code2,
    Cpu,
    KeyRound,
    Server,
    Sparkles
  } from "lucide-svelte";
  import { settingsStore } from "../lib/stores/settings.svelte";
  import { getOpenAiCodexLoginStatus, startOpenAiCodexLogin } from "../lib/tauri";

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

  let codexLoginId = $state<string | null>(null);
  let codexVerificationUri = $state("");
  let codexUserCode = $state("");
  let codexLoginPending = $state(false);
  let codexLoginError = $state<string | null>(null);
  let codexPollTimer: number | null = null;

  function normalizeBackendId(value: string | null) {
    return value?.trim().toLowerCase() ?? providerPresets[0].id;
  }

  function normalizeOptionalText(value: string | null | undefined) {
    const trimmed = value?.trim() ?? "";
    return trimmed.length > 0 ? trimmed : null;
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

<style>
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
    background:
      radial-gradient(circle at top left, rgba(204, 175, 133, 0.28), transparent 38%),
      radial-gradient(circle at bottom right, rgba(89, 124, 109, 0.18), transparent 32%),
      #f5f0e8;
  }

  .configuration-card {
    width: min(960px, 100%);
    padding: 28px;
    border-radius: 28px;
    background: rgba(255, 251, 246, 0.9);
    border: 1px solid rgba(81, 63, 47, 0.08);
    box-shadow: 0 18px 50px rgba(47, 34, 21, 0.12);
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
    color: rgba(78, 64, 51, 0.6);
  }

  h2 {
    margin: 0;
    font-size: 32px;
    line-height: 1.05;
    color: #2d241b;
  }

  .configuration-card-onboarding h2 {
    font-size: 28px;
  }

  .description {
    margin: 10px 0 0;
    max-width: 620px;
    color: rgba(61, 61, 61, 0.72);
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
    border: 1px solid rgba(84, 67, 52, 0.08);
    background: rgba(255, 255, 255, 0.78);
    color: #31271e;
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
    border-color: rgba(93, 128, 112, 0.5);
    background: linear-gradient(180deg, rgba(236, 244, 240, 0.98), rgba(255, 255, 255, 0.98));
    box-shadow: 0 14px 30px rgba(55, 86, 73, 0.12);
  }

  .provider-card strong {
    display: block;
    margin-bottom: 6px;
    font-size: 15px;
  }

  .provider-card p {
    margin: 0;
    color: rgba(61, 61, 61, 0.68);
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
    color: rgba(49, 39, 30, 0.82);
  }

  .field-hint {
    margin: 0;
    font-size: 12px;
    line-height: 1.4;
    color: rgba(61, 61, 61, 0.62);
  }

  .field input {
    width: 100%;
    box-sizing: border-box;
    min-height: 48px;
    padding: 0 14px;
    border-radius: 14px;
    border: 1px solid rgba(86, 68, 52, 0.12);
    background: rgba(255, 255, 255, 0.9);
    color: #31271e;
    font: inherit;
    outline: none;
  }

  .segmented-control {
    display: inline-grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 6px;
    padding: 6px;
    border-radius: 16px;
    background: rgba(255, 255, 255, 0.92);
    border: 1px solid rgba(86, 68, 52, 0.12);
  }

  .segment-button {
    min-height: 40px;
    padding: 0 14px;
    border: none;
    border-radius: 12px;
    background: transparent;
    color: rgba(49, 39, 30, 0.72);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.18s ease, color 0.18s ease, box-shadow 0.18s ease;
  }

  .segment-button.active {
    background: #31271e;
    color: #fffaf4;
    box-shadow: 0 8px 20px rgba(49, 39, 30, 0.16);
  }

  .input-with-icon {
    position: relative;
  }

  .input-with-icon :global(svg) {
    position: absolute;
    top: 16px;
    left: 14px;
    color: rgba(86, 68, 52, 0.48);
  }

  .input-with-icon input {
    padding-left: 40px;
  }

  .oauth-panel {
    margin-top: 18px;
    padding: 16px;
    border-radius: 18px;
    background: rgba(255, 255, 255, 0.74);
    border: 1px solid rgba(84, 67, 52, 0.08);
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
    color: #2d241b;
  }

  .oauth-copy {
    margin: 6px 0 0;
    color: rgba(61, 61, 61, 0.68);
    font-size: 14px;
  }

  .oauth-card {
    margin-top: 16px;
    padding: 14px;
    border-radius: 16px;
    background: rgba(245, 240, 232, 0.9);
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
    background: #2d241b;
    color: #fffaf4;
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
    color: #2e7056;
  }

  .status.pending {
    color: #8b6844;
  }

  .status.error {
    color: #b2483c;
  }

  .subtle {
    margin-top: 4px;
    font-size: 13px;
    color: rgba(61, 61, 61, 0.62);
  }

  .submit-button {
    min-width: 160px;
    min-height: 48px;
    padding: 0 18px;
    border: none;
    border-radius: 14px;
    background: #31271e;
    color: #fffaf4;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  .submit-button.secondary {
    margin-top: 16px;
    background: #4e6558;
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
