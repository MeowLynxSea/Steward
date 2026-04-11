<script lang="ts">
  import { onDestroy } from "svelte";
  import {
    BrainCircuit,
    ChevronLeft,
    Moon,
    Palette,
    Sparkles,
    Sun,
    Waypoints,
    X
  } from "lucide-svelte";
  import { fade, fly } from "svelte/transition";
  import LlmConfigurationPanel from "../components/LlmConfigurationPanel.svelte";
  import MemoryGraphModal from "../components/settings/MemoryGraphModal.svelte";
  import MemorySettingsDrawers from "../components/settings/MemorySettingsDrawers.svelte";
  import MemorySettingsPanel from "../components/settings/MemorySettingsPanel.svelte";
  import {
    memoryItemLabel,
    memoryRouteKey,
    type MemoryNavItem,
    type MemoryPanelMode
  } from "../components/settings/memory";
  import { apiClient } from "../lib/api";
  import { settingsStore } from "../lib/stores/settings.svelte";
  import { themeStore } from "../lib/stores/theme.svelte";
  import type {
    MemoryNodeDetail,
    MemorySearchHit,
    MemorySidebarItem,
    MemorySidebarSection,
    MemoryVersion
  } from "../lib/types";

  type SettingsSection = "general" | "models" | "memory";

  const providerLabels: Record<string, string> = {
    openai: "OpenAI",
    openai_codex: "Codex",
    anthropic: "Anthropic",
    groq: "Groq",
    openrouter: "OpenRouter",
    ollama: "Ollama"
  };

  const embeddingProviderOptions = [
    { value: "openai", label: "OpenAI Compatible" },
    { value: "ollama", label: "Ollama" }
  ];

  const embeddingModelOptions: Record<string, Array<{ value: string; label: string }>> = {
    openai: [
      { value: "text-embedding-3-small", label: "text-embedding-3-small" },
      { value: "text-embedding-3-large", label: "text-embedding-3-large" },
      { value: "text-embedding-ada-002", label: "text-embedding-ada-002" }
    ],
    ollama: [
      { value: "nomic-embed-text", label: "nomic-embed-text" },
      { value: "mxbai-embed-large", label: "mxbai-embed-large" },
      { value: "all-minilm", label: "all-minilm" }
    ]
  };

  function memoryGraphDebug(message: string, payload?: Record<string, unknown>) {
    console.log("[memory-graph][SettingsView]", message, payload ?? {});
  }

  let { onClose }: { onClose: () => void } = $props();

  let activeSection = $state<SettingsSection>("general");
  let showBackendDrawer = $state(false);
  let showMemoryGraphModal = $state(false);
  let showMemoryDrawer = $state(false);
  let memoryDrawerMode = $state<MemoryPanelMode>("node");
  let activeMemoryItem = $state<MemoryNavItem | null>(null);
  let memorySections = $state<MemorySidebarSection[]>([]);
  let selectedNode = $state<MemoryNodeDetail | null>(null);
  let selectedVersions = $state<MemoryVersion[]>([]);
  let memoryPanelLoading = $state(false);
  let memoryError = $state<string | null>(null);
  let memorySearchQuery = $state("");
  let memorySearchResults = $state<MemorySearchHit[]>([]);
  let memorySearchLoading = $state(false);
  let memorySearchHasSearched = $state(false);
  let memorySearchError = $state<string | null>(null);
  let modelSaveTimer: ReturnType<typeof setTimeout> | null = $state(null);

  const backendOptions = $derived(
    settingsStore.data.backends.map((backend) => ({
      value: backend.id,
      label: `${providerLabels[backend.provider] ?? backend.provider} / ${backend.model}`
    }))
  );

  function errorMessage(error: unknown, fallback: string) {
    if (error instanceof Error) {
      return error.message;
    }

    if (typeof error === "string") {
      return error;
    }

    return fallback;
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key !== "Escape") {
      return;
    }

    if (showMemoryDrawer) {
      closeMemoryDrawer();
      return;
    }

    if (showMemoryGraphModal) {
      closeMemoryGraphModal();
      return;
    }

    if (showBackendDrawer) {
      showBackendDrawer = false;
      return;
    }

    onClose();
  }

  function openBackendDrawer() {
    showBackendDrawer = true;
  }

  function closeBackendDrawer() {
    showBackendDrawer = false;
  }

  function closeMemoryDrawer() {
    showMemoryDrawer = false;
    memoryDrawerMode = "node";
    activeMemoryItem = null;
    selectedNode = null;
    selectedVersions = [];
    memoryError = null;
  }

  function openMemoryGraphModal() {
    memoryGraphDebug("openMemoryGraphModal()", {
      activeSection,
      currentValue: showMemoryGraphModal,
      sections: memorySections.length
    });
    showMemoryGraphModal = true;
  }

  function closeMemoryGraphModal() {
    memoryGraphDebug("closeMemoryGraphModal()", {
      activeSection,
      currentValue: showMemoryGraphModal
    });
    showMemoryGraphModal = false;
  }

  async function loadMemoryOverview() {
    memoryPanelLoading = true;
    memoryError = null;
    try {
      const sidebarResponse = await apiClient.listMemorySidebar();
      memorySections = sidebarResponse.sections;
    } catch (error) {
      memoryError = errorMessage(error, "Failed to load memory graph");
    } finally {
      memoryPanelLoading = false;
    }
  }

  async function openMemoryItem(item: MemorySidebarItem) {
    showMemoryDrawer = true;

    activeMemoryItem = {
      key: memoryRouteKey(item),
      title: memoryItemLabel(item),
      description: item.subtitle ?? "检查这个记忆节点的内容、routes、trigger 和版本历史。",
      kind: "node"
    };
    memoryDrawerMode = "node";
    memoryPanelLoading = true;
    memoryError = null;
    selectedNode = null;
    selectedVersions = [];
    try {
      const key = item.uri ?? item.node_id;
      const [detailResponse, versionsResponse] = await Promise.all([
        apiClient.getMemoryNode(key),
        apiClient.getMemoryVersions(key)
      ]);
      selectedNode = detailResponse.detail;
      selectedVersions = versionsResponse.versions;
    } catch (error) {
      memoryError = errorMessage(error, "Failed to load memory node");
    } finally {
      memoryPanelLoading = false;
    }
  }

  function updateMemorySearchQuery(value: string) {
    memorySearchQuery = value;
  }

  async function runMemorySearch() {
    const query = memorySearchQuery.trim();
    if (!query) {
      memorySearchHasSearched = false;
      memorySearchResults = [];
      memorySearchError = null;
      return;
    }

    memorySearchLoading = true;
    memorySearchHasSearched = true;
    memorySearchError = null;

    try {
      const response = await apiClient.searchMemoryGraph(query, 20);
      memorySearchResults = response.results;
    } catch (error) {
      memorySearchError = errorMessage(error, "Failed to search memory graph");
    } finally {
      memorySearchLoading = false;
    }
  }

  async function openMemorySearchDrawer() {
    activeMemoryItem = {
      key: "search",
      title: "Recall Search",
      description: "调试 graph-native 记忆召回结果，查看 route、snippet 和命中节点。",
      kind: "search"
    };
    memoryDrawerMode = "search";
    showMemoryDrawer = true;
    memoryError = null;
  }

  async function openMemoryKey(key: string) {
    const syntheticItem: MemorySidebarItem = {
      node_id: key,
      route_id: null,
      uri: key,
      title: key,
      subtitle: null,
      kind: "reference",
      updated_at: new Date().toISOString()
    };
    await openMemoryItem(syntheticItem);
  }

  function updateEmbeddingProvider(provider: string) {
    const options = embeddingModelOptions[provider] ?? [];
    const nextModel =
      options.find((option) => option.value === settingsStore.data.embeddings.model)?.value ??
      options[0]?.value ??
      settingsStore.data.embeddings.model;
    settingsStore.setEmbeddings({ provider, model: nextModel });
    scheduleModelSettingsSave();
  }

  function updateEmbeddingDimension(rawValue: string) {
    const value = rawValue.trim();
    const parsed = Number.parseInt(value, 10);
    settingsStore.setEmbeddings({
      dimension: value === "" || !Number.isSafeInteger(parsed) || parsed <= 0 ? null : parsed
    });
    scheduleModelSettingsSave();
  }

  async function saveModelSettings() {
    if (modelSaveTimer !== null) {
      clearTimeout(modelSaveTimer);
      modelSaveTimer = null;
    }
    await settingsStore.save();
  }

  function scheduleModelSettingsSave(delay = 360) {
    if (modelSaveTimer !== null) {
      clearTimeout(modelSaveTimer);
    }
    settingsStore.error = null;
    settingsStore.status = "正在保存...";
    modelSaveTimer = setTimeout(() => {
      modelSaveTimer = null;
      void saveModelSettings();
    }, delay);
  }

  function updateEmbeddings(patch: Partial<typeof settingsStore.data.embeddings>) {
    settingsStore.setEmbeddings(patch);
    scheduleModelSettingsSave();
  }

  function updateMajorBackend(value: string) {
    settingsStore.setMajorBackend(value || null);
    scheduleModelSettingsSave();
  }

  function updateCheapBackend(value: string) {
    settingsStore.setCheapBackend(value || null);
    scheduleModelSettingsSave();
  }

  function updateCheapModelUsesPrimary(checked: boolean) {
    settingsStore.setCheapModelUsesPrimary(checked);
    scheduleModelSettingsSave();
  }

  function selectSection(section: SettingsSection) {
    memoryGraphDebug("selectSection()", {
      from: activeSection,
      to: section,
      showMemoryGraphModal
    });
    activeSection = section;

    if (section !== "memory") {
      closeMemoryDrawer();
      closeMemoryGraphModal();
    } else if (memorySections.length === 0 && !memoryPanelLoading) {
      void loadMemoryOverview();
    }

    if (section !== "models") {
      closeBackendDrawer();
    }
  }

  const activeEmbeddingModelOptions = $derived(
    embeddingModelOptions[settingsStore.data.embeddings.provider] ?? []
  );

  onDestroy(() => {
    if (modelSaveTimer !== null) {
      clearTimeout(modelSaveTimer);
      modelSaveTimer = null;
      void settingsStore.save();
    }
  });

  $effect(() => {
    memoryGraphDebug("showMemoryGraphModal changed", {
      value: showMemoryGraphModal,
      activeSection
    });
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<div
  class="drawer-backdrop"
  transition:fade={{ duration: 200 }}
  role="presentation"
  onclick={onClose}
></div>

<div
  class="settings-drawer"
  in:fly={{ x: -420, duration: 280, easing: (t) => 1 - Math.pow(1 - t, 3) }}
  out:fly={{ x: -420, duration: 220, easing: (t) => t * t }}
>
  <div class="drawer-header">
    <div class="header-left">
      <p class="header-eyebrow">Settings</p>
      <h3>设置</h3>
    </div>
    <button class="close-btn" onclick={onClose} aria-label="关闭">
      <X size={18} strokeWidth={2} />
    </button>
  </div>

  <div class="nav-tabs" role="tablist">
    <button
      class:selected={activeSection === "general"}
      class="nav-tab"
      role="tab"
      aria-selected={activeSection === "general"}
      onclick={() => selectSection("general")}
    >
      <Palette size={15} strokeWidth={2} />
      <span>常规</span>
    </button>
    <button
      class:selected={activeSection === "memory"}
      class="nav-tab"
      role="tab"
      aria-selected={activeSection === "memory"}
      onclick={() => selectSection("memory")}
    >
      <Waypoints size={15} strokeWidth={2} />
      <span>记忆</span>
    </button>
    <button
      class:selected={activeSection === "models"}
      class="nav-tab"
      role="tab"
      aria-selected={activeSection === "models"}
      onclick={() => selectSection("models")}
    >
      <Sparkles size={15} strokeWidth={2} />
      <span>模型</span>
    </button>
  </div>

  <div class="drawer-content">
    {#if activeSection === "general"}
      <section class="settings-section">
        <div class="section-header">
          <h4>外观</h4>
          <p>标题栏和设置页都可以切换主题，模式会立即保存在当前设备。</p>
        </div>

        <div class="settings-card">
          <div class="card-copy">
            <span class="card-kicker">Theme</span>
            <h3>明暗模式</h3>
            <p>保留顶部标题栏切换，同时也可以在这里直接选择浅色或深色。</p>
          </div>

          <div class="theme-toggle-group" role="group" aria-label="主题">
            <button
              class:active={themeStore.mode === "light"}
              class="theme-option"
              type="button"
              onclick={() => themeStore.setMode("light")}
            >
              <Sun size={15} strokeWidth={2} />
              <span>浅色</span>
            </button>
            <button
              class:active={themeStore.mode === "dark"}
              class="theme-option"
              type="button"
              onclick={() => themeStore.setMode("dark")}
            >
              <Moon size={15} strokeWidth={2} />
              <span>深色</span>
            </button>
          </div>
        </div>
      </section>
    {:else if activeSection === "memory"}
      <MemorySettingsPanel
        {memoryError}
        onOpenGraph={openMemoryGraphModal}
        onOpenSearch={openMemorySearchDrawer}
      />
    {:else}
      <section class="settings-section">
        <div class="section-header">
          <h4>模型设置</h4>
          <p>配置多个后端，Major 和 Cheap 模型各选一个。</p>
        </div>

        <button class="settings-card settings-card-button" type="button" onclick={openBackendDrawer}>
          <div class="card-copy">
            <span class="card-kicker">Backend 管理</span>
            <h3>管理可用模型</h3>
            <p>添加、编辑、删除后端配置。</p>
          </div>
          <ChevronLeft size={18} strokeWidth={2} class="chevron-left" />
        </button>

        <div class="settings-card model-selectors">
          <label class="model-select">
            <span class="model-select-label">主模型</span>
            <select
              class="model-select-input"
              value={settingsStore.data.major_backend_id ?? ""}
              onchange={(event) =>
                updateMajorBackend((event.currentTarget as HTMLSelectElement).value)
              }
            >
              <option value="">选择主模型...</option>
              {#each backendOptions as option (option.value)}
                <option value={option.value}>{option.label}</option>
              {/each}
            </select>
          </label>

          <label class="model-select">
            <span class="model-select-label">Cheap 模型</span>
            <div class="cheap-toggle-row">
              <label class="checkbox-row">
                <input
                  type="checkbox"
                  checked={settingsStore.data.cheap_model_uses_primary}
                  onchange={(event) =>
                    updateCheapModelUsesPrimary(
                      (event.currentTarget as HTMLInputElement).checked
                    )}
                />
                <span>使用主模型</span>
              </label>
            </div>
            {#if !settingsStore.data.cheap_model_uses_primary}
              <select
                class="model-select-input"
                value={settingsStore.data.cheap_backend_id ?? ""}
                onchange={(event) =>
                  updateCheapBackend((event.currentTarget as HTMLSelectElement).value)
                }
              >
                <option value="">选择 Cheap 模型...</option>
                {#each backendOptions as option (option.value)}
                  <option value={option.value}>{option.label}</option>
                {/each}
              </select>
            {/if}
          </label>
        </div>

        <div class="settings-card retrieval-settings">
          <div class="card-copy">
            <span class="card-kicker">Embeddings</span>
            <h3>语义召回模型</h3>
            <p>这套配置只用于 embedding / recall，不会替换主聊天模型或 Cheap 模型。</p>
          </div>

          <label class="checkbox-row toggle-row">
            <input
              type="checkbox"
              checked={settingsStore.data.embeddings.enabled}
              onchange={(event) =>
                updateEmbeddings({
                  enabled: (event.currentTarget as HTMLInputElement).checked
                })}
            />
            <span>启用 embeddings</span>
          </label>

          <div class="model-selectors embeddings-grid">
            <label class="model-select">
              <span class="model-select-label">Provider</span>
              <select
                class="model-select-input"
                value={settingsStore.data.embeddings.provider}
                onchange={(event) =>
                  updateEmbeddingProvider(
                    (event.currentTarget as HTMLSelectElement).value
                  )}
              >
                {#each embeddingProviderOptions as option (option.value)}
                  <option value={option.value}>{option.label}</option>
                {/each}
              </select>
            </label>

            <label class="model-select">
              <span class="model-select-label">Model ID</span>
              <input
                class="model-text-input"
                type="text"
                value={settingsStore.data.embeddings.model}
                list="embedding-model-suggestions"
                oninput={(event) =>
                  updateEmbeddings({
                    model: (event.currentTarget as HTMLInputElement).value
                  })}
                placeholder="text-embedding-3-small"
              />
              <datalist id="embedding-model-suggestions">
                {#each activeEmbeddingModelOptions as option (option.value)}
                  <option value={option.value}>{option.label}</option>
                {/each}
              </datalist>
            </label>

            <label class="model-select">
              <span class="model-select-label">Base URL</span>
              <input
                class="model-text-input"
                type="text"
                value={settingsStore.data.embeddings.base_url ?? ""}
                oninput={(event) =>
                  updateEmbeddings({
                    base_url: (event.currentTarget as HTMLInputElement).value || null
                  })}
                placeholder={
                  settingsStore.data.embeddings.provider === "ollama"
                    ? "http://127.0.0.1:11434"
                    : "https://api.openai.com"
                }
              />
              <p class="field-hint">
                {#if settingsStore.data.embeddings.provider === "ollama"}
                  填 Ollama 服务根地址，例如 `http://127.0.0.1:11434`。
                {:else}
                  填 OpenAI-compatible 服务根地址，不要带 `/v1`。程序会自动请求
                  `/v1/embeddings`。例如应填写 `https://api.siliconflow.cn`，不要填写
                  `https://api.siliconflow.cn/v1`。
                {/if}
              </p>
            </label>

            <label class="model-select">
              <span class="model-select-label">Dimensions</span>
              <input
                class="model-text-input"
                type="number"
                min="1"
                step="1"
                value={settingsStore.data.embeddings.dimension ?? ""}
                oninput={(event) =>
                  updateEmbeddingDimension((event.currentTarget as HTMLInputElement).value)}
                placeholder="留空则按模型默认值推断"
              />
              <p class="field-hint">
                只有第三方 embedding 服务要求固定维度时才填写；多数情况下可留空，由模型默认值自动推断。
              </p>
            </label>

            <label class="model-select">
              <span class="model-select-label">API Key</span>
              <input
                class="model-text-input"
                type="password"
                value={settingsStore.data.embeddings.api_key ?? ""}
                oninput={(event) =>
                  updateEmbeddings({
                    api_key: (event.currentTarget as HTMLInputElement).value || null
                  })}
                placeholder={
                  settingsStore.data.embeddings.provider === "ollama"
                    ? "可留空"
                    : "sk-..."
                }
              />
            </label>
          </div>

          <div class="retrieval-notes">
            <div class="note-chip">
              <BrainCircuit size={14} strokeWidth={2} />
              <span>支持 OpenAI-compatible 第三方 embedding 服务。</span>
            </div>
            <p>
              例如把 provider 设为 `OpenAI Compatible`，再填写第三方的 `Base URL`、`Model ID`
              与 `API Key`。保存后会热更新当前运行时，并在后台回填向量。
            </p>
          </div>
        </div>

        <div class="settings-actions">
          {#if settingsStore.status}
            <p class="settings-status">{settingsStore.status}</p>
          {/if}
          {#if settingsStore.error}
            <p class="settings-status error">{settingsStore.error}</p>
          {/if}
        </div>
      </section>
    {/if}
  </div>

  {#if showBackendDrawer}
    <div
      class="nested-backdrop"
      transition:fade={{ duration: 180 }}
      role="presentation"
      onclick={closeBackendDrawer}
    ></div>

    <div
      class="backend-drawer"
      in:fly={{ x: -420, duration: 280, easing: (t) => 1 - Math.pow(1 - t, 3) }}
      out:fly={{ x: -420, duration: 220, easing: (t) => t * t }}
    >
      <div class="drawer-header nested-header">
        <button class="back-btn" onclick={closeBackendDrawer} aria-label="返回">
          <ChevronLeft size={18} strokeWidth={2} />
        </button>
        <div class="header-center">
          <p class="header-eyebrow">Backend</p>
          <h3>管理可用模型</h3>
        </div>
        <div class="header-spacer"></div>
      </div>

      <div class="drawer-content nested-content">
        <LlmConfigurationPanel mode="settings" />
      </div>
    </div>
  {/if}
</div>

{#if showMemoryGraphModal}
  <MemoryGraphModal
    {memorySections}
    onClose={closeMemoryGraphModal}
  />
{/if}

<MemorySettingsDrawers
  {showMemoryDrawer}
  {memoryDrawerMode}
  {activeMemoryItem}
  {selectedNode}
  {selectedVersions}
  {memoryPanelLoading}
  {memoryError}
  {memorySearchQuery}
  {memorySearchResults}
  {memorySearchLoading}
  {memorySearchHasSearched}
  {memorySearchError}
  onCloseMemoryDrawer={closeMemoryDrawer}
  onMemorySearchQueryChange={updateMemorySearchQuery}
  onRunMemorySearch={runMemorySearch}
  onOpenMemoryKey={openMemoryKey}
/>

<style>
  .drawer-backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: rgba(0, 0, 0, 0.2);
    backdrop-filter: blur(12px);
  }

  .settings-drawer {
    position: fixed;
    top: 0;
    left: 0;
    bottom: 0;
    z-index: 41;
    width: min(420px, 100vw);
    display: flex;
    flex-direction: column;
    background: var(--bg-surface);
    border-right: 1px solid var(--border-default);
    box-shadow: var(--shadow-dropdown);
    overflow: hidden;
  }

  .drawer-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 18px 16px 14px;
    border-bottom: 1px solid var(--border-default);
  }

  .header-left {
    min-width: 0;
  }

  .header-eyebrow {
    margin: 0 0 4px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .drawer-header h3 {
    margin: 0;
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .close-btn {
    width: 36px;
    height: 36px;
    border-radius: 10px;
    border: none;
    background: var(--bg-hover);
    color: var(--text-secondary);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s ease, color 0.15s ease, transform 0.15s ease;
  }

  .close-btn:hover {
    background: var(--bg-active);
    color: var(--text-primary);
    transform: translateY(-1px);
  }

  .nav-tabs {
    display: flex;
    gap: 8px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border-default);
  }

  .nav-tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    border-radius: 12px;
    border: 1px solid transparent;
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .nav-tab:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .nav-tab.selected {
    background: var(--bg-elevated);
    border-color: var(--border-default);
    color: var(--text-primary);
  }

  .drawer-content {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 18px 16px;
  }

  .settings-section {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .section-header h4 {
    margin: 0;
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .section-header p {
    margin: 6px 0 0;
    color: var(--text-secondary);
    font-size: 13px;
    line-height: 1.5;
  }

  .settings-card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    padding: 16px;
    border-radius: 18px;
    border: 1px solid var(--border-default);
    background: var(--bg-surface);
  }

  .theme-toggle-group {
    display: inline-flex;
    gap: 6px;
    padding: 6px;
    border-radius: 16px;
    background: var(--bg-input);
    border: 1px solid var(--border-input, var(--border-default));
    flex-shrink: 0;
  }

  .theme-option {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    border: none;
    border-radius: 12px;
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .theme-option:hover {
    color: var(--text-primary);
  }

  .theme-option.active {
    background: var(--accent-primary);
    color: var(--text-on-dark);
    box-shadow: var(--shadow-card);
  }

  .settings-card-button {
    width: 100%;
    text-align: left;
    cursor: pointer;
    color: inherit;
    transition: all 0.15s ease;
  }

  .settings-card-button:hover {
    border-color: color-mix(in srgb, var(--accent-gold) 30%, var(--border-default));
    background: color-mix(in srgb, var(--bg-surface) 95%, var(--bg-elevated) 5%);
    transform: translateY(-1px);
  }

  .card-copy {
    min-width: 0;
    flex: 1;
  }

  .card-kicker {
    display: inline-block;
    margin-bottom: 4px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .card-copy h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    color: var(--text-primary);
  }

  .card-copy p {
    margin: 4px 0 0;
    color: var(--text-secondary);
    font-size: 12px;
    line-height: 1.45;
  }

  :global(.chevron-left) {
    color: var(--text-tertiary);
    transform: rotate(180deg);
    flex-shrink: 0;
  }

  .model-selectors {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 16px;
  }

  .retrieval-settings {
    align-items: stretch;
    flex-direction: column;
  }

  .embeddings-grid {
    width: 100%;
  }

  .model-select {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .model-select-label {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .model-select-input {
    width: 100%;
    height: 40px;
    padding: 0 14px;
    padding-right: 38px;
    border-radius: 12px;
    border: 1px solid var(--border-input);
    background: var(--bg-input);
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%23888' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpolyline points='6 9 12 15 18 9'%3E%3C/polyline%3E%3C/svg%3E");
    background-position: right 14px center;
    background-repeat: no-repeat;
    color: var(--text-primary);
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    appearance: none;
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }

  .model-select-input:hover {
    border-color: var(--border-focus, var(--text-tertiary));
  }

  .model-select-input:focus {
    outline: none;
    border-color: var(--accent-primary);
    box-shadow: 0 0 0 3px
      color-mix(in srgb, var(--accent-primary) 20%, transparent);
  }

  .model-text-input {
    width: 100%;
    height: 40px;
    padding: 0 14px;
    border-radius: 12px;
    border: 1px solid var(--border-input);
    background: var(--bg-input);
    color: var(--text-primary);
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }

  .model-text-input:hover {
    border-color: var(--border-focus, var(--text-tertiary));
  }

  .model-text-input:focus {
    outline: none;
    border-color: var(--accent-primary);
    box-shadow: 0 0 0 3px
      color-mix(in srgb, var(--accent-primary) 20%, transparent);
  }

  .cheap-toggle-row {
    margin-bottom: 4px;
  }

  .toggle-row {
    align-self: flex-start;
  }

  .checkbox-row {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 500;
    color: var(--text-secondary);
  }

  .checkbox-row input[type="checkbox"] {
    width: 16px;
    height: 16px;
    border-radius: 6px;
    border: 1px solid var(--border-input);
    background: var(--bg-input);
    cursor: pointer;
    accent-color: var(--accent-primary);
  }

  .retrieval-notes {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
    border-radius: 14px;
    background: color-mix(in srgb, var(--bg-elevated) 75%, var(--bg-surface) 25%);
    border: 1px solid var(--border-subtle, var(--border-default));
  }

  .retrieval-notes p {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-secondary);
  }

  .note-chip {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    color: var(--text-primary);
    font-size: 12px;
    font-weight: 600;
  }

  .settings-actions {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
  }

  .settings-status {
    margin: 0;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .settings-status.error {
    color: var(--accent-error, #c65a50);
  }

  .nested-backdrop {
    position: absolute;
    inset: 0;
    z-index: 42;
    background: rgba(0, 0, 0, 0.15);
    backdrop-filter: blur(8px);
  }

  .backend-drawer {
    position: absolute;
    inset: 0;
    z-index: 43;
    display: flex;
    flex-direction: column;
    background: var(--bg-surface);
    box-shadow: 2px 0 20px rgba(0, 0, 0, 0.15);
  }

  .nested-header {
    padding: 18px 16px 14px;
  }

  .back-btn {
    width: 36px;
    height: 36px;
    border-radius: 10px;
    border: none;
    background: var(--bg-hover);
    color: var(--text-secondary);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: all 0.15s ease;
    flex-shrink: 0;
  }

  .back-btn:hover {
    background: var(--bg-active);
    color: var(--text-primary);
    transform: translateY(-1px);
  }

  .header-center {
    flex: 1;
    text-align: center;
    min-width: 0;
  }

  .header-spacer {
    width: 36px;
    flex-shrink: 0;
  }

  .nested-content {
    padding: 16px;
  }

  @media (max-width: 640px) {
    .settings-drawer {
      width: 100vw;
    }
  }
</style>
