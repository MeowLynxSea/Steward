<script lang="ts">
  import { ChevronRight, Moon, Palette, Sparkles, Sun, X } from "lucide-svelte";
  import LlmConfigurationPanel from "../components/LlmConfigurationPanel.svelte";
  import { settingsStore } from "../lib/stores/settings.svelte";
  import { themeStore } from "../lib/stores/theme.svelte";

  type SettingsSection = "general";

  const providerLabels: Record<string, string> = {
    openai: "OpenAI",
    openai_codex: "Codex",
    anthropic: "Anthropic",
    groq: "Groq",
    openrouter: "OpenRouter",
    ollama: "Ollama"
  };

  let { onClose }: { onClose: () => void } = $props();

  let activeSection = $state<SettingsSection>("general");
  let showModelSettings = $state(false);

  const currentProviderLabel = $derived.by(() => {
    const backend = settingsStore.data.llm_backend?.trim().toLowerCase();
    if (!backend) {
      return "未选择";
    }
    return providerLabels[backend] ?? backend;
  });

  const cheapModelSummary = $derived.by(() => {
    if (settingsStore.data.cheap_model_uses_primary) {
      return "使用主模型";
    }
    return settingsStore.data.cheap_model?.trim() || "未设置";
  });

  function handleKeydown(event: KeyboardEvent) {
    if (event.key !== "Escape") {
      return;
    }

    if (showModelSettings) {
      showModelSettings = false;
      return;
    }

    onClose();
  }

  function handleMainBackdropClick(event: MouseEvent) {
    if ((event.target as HTMLElement).classList.contains("settings-backdrop")) {
      onClose();
    }
  }

  function handleNestedBackdropClick(event: MouseEvent) {
    if ((event.target as HTMLElement).classList.contains("model-settings-layer")) {
      showModelSettings = false;
    }
  }

  function openModelSettings() {
    showModelSettings = true;
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="settings-backdrop" onclick={handleMainBackdropClick}>
  <div class:layered={showModelSettings} class="settings-modal" role="dialog" aria-modal="true" aria-label="设置">
    <div class="settings-modal-header">
      <div class="settings-modal-copy">
        <p class="settings-modal-eyebrow">Settings</p>
        <h3 class="settings-modal-title">设置</h3>
      </div>

      <button class="settings-close-btn" onclick={onClose} aria-label="关闭设置">
        <X size={18} strokeWidth={2} />
      </button>
    </div>

    <div class="settings-layout">
      <aside class="settings-sidebar" aria-label="设置分组">
        <button
          class:selected={activeSection === "general"}
          class="settings-nav-button"
          type="button"
          onclick={() => activeSection = "general"}
        >
          <Palette size={16} strokeWidth={2} />
          <span>常规</span>
        </button>

        <button class="settings-nav-button" type="button" onclick={openModelSettings}>
          <Sparkles size={16} strokeWidth={2} />
          <span>模型</span>
        </button>
      </aside>

      <div class="settings-content">
        {#if activeSection === "general"}
          <section class="settings-section">
            <div class="section-header">
              <p class="section-eyebrow">General</p>
              <h4>常规设置</h4>
              <p>简单项直接在这里修改，复杂模型配置单独打开新的模态框。</p>
            </div>

            <div class="settings-card">
              <div class="card-copy">
                <h5>外观</h5>
                <p>选择应用主题。这个设置仅保存在当前设备。</p>
              </div>

              <div class="theme-toggle-group" role="group" aria-label="主题">
                <button
                  class:active={themeStore.mode === "light"}
                  class="theme-option"
                  type="button"
                  onclick={() => themeStore.setMode("light")}
                >
                  <Sun size={16} strokeWidth={2} />
                  <span>浅色</span>
                </button>

                <button
                  class:active={themeStore.mode === "dark"}
                  class="theme-option"
                  type="button"
                  onclick={() => themeStore.setMode("dark")}
                >
                  <Moon size={16} strokeWidth={2} />
                  <span>深色</span>
                </button>
              </div>
            </div>

            <button class="settings-card settings-card-button" type="button" onclick={openModelSettings}>
              <div class="card-copy">
                <span class="card-kicker">Model Configuration</span>
                <h5>打开模型设置</h5>
                <p>主模型、Cheap LLM、API Key、Base URL 等详细选项都在二级模态框里处理。</p>
                <div class="card-meta">
                  <span>当前提供商：{currentProviderLabel}</span>
                  <span>Cheap LLM：{cheapModelSummary}</span>
                </div>
              </div>

              <ChevronRight size={18} strokeWidth={2} />
            </button>
          </section>
        {/if}
      </div>
    </div>
  </div>

  {#if showModelSettings}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="model-settings-layer" onclick={handleNestedBackdropClick}>
      <div class="nested-settings-modal" role="dialog" aria-modal="true" aria-label="模型设置">
        <div class="nested-settings-header">
          <div>
            <p class="nested-settings-eyebrow">Models</p>
            <h4>模型设置</h4>
          </div>

          <button class="settings-close-btn" type="button" onclick={() => showModelSettings = false} aria-label="关闭模型设置">
            <X size={18} strokeWidth={2} />
          </button>
        </div>

        <div class="nested-settings-body">
          <LlmConfigurationPanel
            mode="settings"
            eyebrow="模型设置"
            title="模型与提供商"
            description="详细模型参数放在这里，避免主设置页塞入过多内容。Cheap LLM 默认可跟随主模型。"
            submitLabel="保存更改"
          />
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .settings-backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: rgba(0, 0, 0, 0.35);
    backdrop-filter: blur(12px);
    animation: fadeIn 0.2s ease;
  }

  @keyframes fadeIn {
    from {
      opacity: 0;
    }

    to {
      opacity: 1;
    }
  }

  .settings-modal {
    position: relative;
    z-index: 100;
    width: min(100%, 940px);
    max-height: calc(100vh - 48px);
    border-radius: 26px;
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
    box-shadow: var(--shadow-dropdown);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    transition: transform 0.2s ease, opacity 0.2s ease, filter 0.2s ease;
  }

  .settings-modal.layered {
    transform: scale(0.975) translateY(12px);
    opacity: 0.56;
    filter: blur(1.5px);
    pointer-events: none;
  }

  .settings-modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 20px 22px 18px;
    border-bottom: 1px solid var(--border-default);
  }

  .settings-modal-copy {
    min-width: 0;
  }

  .settings-modal-eyebrow,
  .section-eyebrow,
  .nested-settings-eyebrow,
  .card-kicker {
    margin: 0 0 6px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .settings-modal-title,
  .section-header h4,
  .nested-settings-header h4,
  .card-copy h5 {
    margin: 0;
    color: var(--text-primary);
  }

  .settings-modal-title {
    font-size: 20px;
    font-weight: 700;
  }

  .settings-close-btn {
    width: 36px;
    height: 36px;
    border-radius: 10px;
    background: transparent;
    border: none;
    color: var(--text-tertiary);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s ease, color 0.15s ease;
  }

  .settings-close-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .settings-layout {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 220px minmax(0, 1fr);
  }

  .settings-sidebar {
    padding: 18px;
    border-right: 1px solid var(--border-default);
    background: color-mix(in srgb, var(--bg-sidebar) 72%, transparent);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .settings-nav-button {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    min-height: 42px;
    padding: 0 14px;
    border-radius: 14px;
    border: 1px solid transparent;
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease, border-color 0.15s ease;
  }

  .settings-nav-button:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .settings-nav-button.selected {
    background: var(--bg-elevated);
    border-color: var(--border-default);
    color: var(--text-primary);
  }

  .settings-content {
    min-height: 0;
    overflow-y: auto;
    padding: 26px;
  }

  .settings-section {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .section-header p,
  .card-copy p {
    margin: 8px 0 0;
    color: var(--text-secondary);
    font-size: 14px;
    line-height: 1.55;
  }

  .settings-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 18px;
    padding: 18px;
    border-radius: 20px;
    border: 1px solid var(--border-default);
    background: color-mix(in srgb, var(--bg-surface) 88%, var(--bg-elevated) 12%);
  }

  .settings-card-button {
    width: 100%;
    text-align: left;
    cursor: pointer;
    color: inherit;
  }

  .settings-card-button:hover {
    border-color: color-mix(in srgb, var(--accent-primary) 28%, var(--border-default));
    background: color-mix(in srgb, var(--bg-surface) 82%, var(--bg-elevated) 18%);
  }

  .card-copy {
    min-width: 0;
  }

  .card-kicker {
    display: inline-block;
  }

  .card-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin-top: 14px;
  }

  .card-meta span {
    display: inline-flex;
    align-items: center;
    min-height: 28px;
    padding: 0 10px;
    border-radius: 999px;
    background: var(--bg-elevated);
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 600;
  }

  .theme-toggle-group {
    display: inline-grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
    padding: 6px;
    border-radius: 16px;
    background: var(--bg-input);
    border: 1px solid var(--border-input);
  }

  .theme-option {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    min-width: 110px;
    min-height: 40px;
    padding: 0 14px;
    border: none;
    border-radius: 12px;
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease, box-shadow 0.15s ease;
  }

  .theme-option.active {
    background: var(--accent-primary);
    color: var(--text-on-dark);
    box-shadow: var(--shadow-card);
  }

  .model-settings-layer {
    position: fixed;
    inset: 0;
    z-index: 120;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 36px;
    background: rgba(8, 10, 16, 0.32);
    backdrop-filter: blur(6px);
  }

  .nested-settings-modal {
    width: min(100%, 860px);
    max-height: calc(100vh - 96px);
    border-radius: 24px;
    border: 1px solid var(--border-default);
    background: var(--bg-surface);
    box-shadow: var(--shadow-dropdown);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .nested-settings-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 18px 20px 16px;
    border-bottom: 1px solid var(--border-default);
  }

  .nested-settings-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 18px 20px 20px;
  }

  @media (max-width: 840px) {
    .settings-backdrop,
    .model-settings-layer {
      padding: 14px;
    }

    .settings-layout {
      grid-template-columns: 1fr;
    }

    .settings-sidebar {
      flex-direction: row;
      border-right: none;
      border-bottom: 1px solid var(--border-default);
      overflow-x: auto;
    }

    .settings-nav-button {
      width: auto;
      min-width: 120px;
      justify-content: center;
    }

    .settings-content {
      padding: 18px;
    }

    .settings-card {
      flex-direction: column;
      align-items: stretch;
    }

    .theme-toggle-group {
      width: 100%;
    }

    .theme-option {
      min-width: 0;
    }
  }
</style>
