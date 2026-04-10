<script lang="ts">
  import {
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
  import MemorySettingsDrawers from "../components/settings/MemorySettingsDrawers.svelte";
  import MemorySettingsPanel from "../components/settings/MemorySettingsPanel.svelte";
  import {
    memoryRouteKey,
    type MemoryNavItem,
    type MemoryPanelMode
  } from "../components/settings/memory";
  import { apiClient } from "../lib/api";
  import { settingsStore } from "../lib/stores/settings.svelte";
  import { themeStore } from "../lib/stores/theme.svelte";
  import type {
    MemoryChangeSet,
    MemoryNodeDetail,
    MemorySearchHit,
    MemorySidebarItem,
    MemorySidebarSection,
    MemoryTimelineEntry,
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

  let { onClose }: { onClose: () => void } = $props();

  let activeSection = $state<SettingsSection>("general");
  let showBackendDrawer = $state(false);
  let showMemoryDrawer = $state(false);
  let memoryDrawerMode = $state<MemoryPanelMode>("node");
  let activeMemoryItem = $state<MemoryNavItem | null>(null);
  let memorySections = $state<MemorySidebarSection[]>([]);
  let memoryTimeline = $state<MemoryTimelineEntry[]>([]);
  let memoryReviews = $state<MemoryChangeSet[]>([]);
  let selectedNode = $state<MemoryNodeDetail | null>(null);
  let selectedVersions = $state<MemoryVersion[]>([]);
  let memoryPanelLoading = $state(false);
  let memoryError = $state<string | null>(null);
  let memorySearchQuery = $state("");
  let memorySearchResults = $state<MemorySearchHit[]>([]);
  let memorySearchLoading = $state(false);
  let memorySearchHasSearched = $state(false);
  let memorySearchError = $state<string | null>(null);

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

  async function loadMemoryOverview() {
    memoryPanelLoading = true;
    memoryError = null;
    try {
      const [sidebarResponse, timelineResponse, reviewsResponse] = await Promise.all([
        apiClient.listMemorySidebar(),
        apiClient.listMemoryTimeline(),
        apiClient.listMemoryReviews()
      ]);
      memorySections = sidebarResponse.sections;
      memoryTimeline = timelineResponse.entries;
      memoryReviews = reviewsResponse.reviews;
    } catch (error) {
      memoryError = errorMessage(error, "Failed to load memory graph");
    } finally {
      memoryPanelLoading = false;
    }
  }

  async function openMemoryItem(item: MemorySidebarItem) {
    showMemoryDrawer = true;
    if (item.uri?.startsWith("review://")) {
      activeMemoryItem = {
        key: item.uri,
        title: "Review Queue",
        description: "检查 AI 对记忆图谱的结构化修改，并决定接受还是回滚。",
        kind: "reviews"
      };
      memoryDrawerMode = "reviews";
      return;
    }

    activeMemoryItem = {
      key: memoryRouteKey(item),
      title: item.title,
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

  async function openMemoryReviewsDrawer() {
    activeMemoryItem = {
      key: "reviews",
      title: "Review Queue",
      description: "查看待审查的 changeset，并决定接受还是回滚。",
      kind: "reviews"
    };
    memoryDrawerMode = "reviews";
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

  async function handleApplyMemoryReview(id: string, action: "accept" | "rollback") {
    const response =
      action === "rollback"
        ? await apiClient.rollbackMemoryChangeset(id)
        : await apiClient.applyMemoryReview(id, action);
    memoryReviews = response.reviews;
  }

  function selectSection(section: SettingsSection) {
    activeSection = section;

    if (section !== "memory") {
      closeMemoryDrawer();
    } else if (memorySections.length === 0 && !memoryPanelLoading) {
      void loadMemoryOverview();
    }

    if (section !== "models") {
      closeBackendDrawer();
    }
  }
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
          <p>选择应用主题，仅保存在当前设备。</p>
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
      </section>
    {:else if activeSection === "memory"}
      <MemorySettingsPanel
        {memorySections}
        {memoryTimeline}
        {memoryReviews}
        {memoryError}
        onOpenItem={openMemoryItem}
        onOpenSearch={openMemorySearchDrawer}
        onOpenReviews={openMemoryReviewsDrawer}
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
                settingsStore.setMajorBackend(
                  (event.currentTarget as HTMLSelectElement).value || null
                )}
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
                  onchange={(event) => {
                    settingsStore.data.cheap_model_uses_primary = (
                      event.currentTarget as HTMLInputElement
                    ).checked;
                  }}
                />
                <span>使用主模型</span>
              </label>
            </div>
            {#if !settingsStore.data.cheap_model_uses_primary}
              <select
                class="model-select-input"
                value={settingsStore.data.cheap_backend_id ?? ""}
                onchange={(event) =>
                  settingsStore.setCheapBackend(
                    (event.currentTarget as HTMLSelectElement).value || null
                  )}
              >
                <option value="">选择 Cheap 模型...</option>
                {#each backendOptions as option (option.value)}
                  <option value={option.value}>{option.label}</option>
                {/each}
              </select>
            {/if}
          </label>
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

<MemorySettingsDrawers
  {showMemoryDrawer}
  {memoryDrawerMode}
  {activeMemoryItem}
  {selectedNode}
  {selectedVersions}
  {memoryReviews}
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
  onApplyMemoryReview={handleApplyMemoryReview}
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

  .theme-toggle-group {
    display: inline-flex;
    gap: 6px;
    padding: 6px;
    border-radius: 16px;
    background: var(--bg-input);
    border: 1px solid var(--border-input);
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
    flex-direction: column;
    align-items: stretch;
    gap: 16px;
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

  .cheap-toggle-row {
    margin-bottom: 4px;
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
