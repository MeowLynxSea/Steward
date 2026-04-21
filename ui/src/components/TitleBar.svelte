<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { Check, ChevronDown, ChevronLeft, ChevronRight, Minus, Moon, Settings, Square, Sun, X } from "lucide-svelte";
  import type { SessionSummary } from "../lib/types";
  import { themeStore } from "../lib/stores/theme.svelte";

  type ModelOption = {
    value: string;
    label: string;
    model: string;
  };

  interface Props {
    title?: string;
    session?: SessionSummary | null;
    leftSidebarCollapsed?: boolean;
    rightSidebarCollapsed?: boolean;
    onToggleLeft?: () => void;
    onToggleRight?: () => void;
    availableModels?: ModelOption[];
    selectedModelValue?: string;
    onSelectModel?: (model: string) => void;
    onOpenSettings?: () => void;
  }

  let {
    title = "Steward",
    session = null,
    leftSidebarCollapsed = false,
    rightSidebarCollapsed = false,
    onToggleLeft,
    onToggleRight,
    availableModels = [],
    selectedModelValue = "",
    onSelectModel,
    onOpenSettings
  }: Props = $props();

  const appWindow = getCurrentWindow();
  const isMac = navigator.platform.toLowerCase().includes("mac");
  let showModelDropdown = $state(false);
  const darkMode = $derived(themeStore.mode === "dark");

  async function minimize() {
    await appWindow.minimize();
  }

  async function maximize() {
    await appWindow.toggleMaximize();
  }

  async function close() {
    await appWindow.close();
  }

  function isInteractiveTitlebarTarget(target: EventTarget | null): boolean {
    if (!(target instanceof HTMLElement)) return false;

    return Boolean(
      target.closest(
        "button, input, select, textarea, a, .model-selector, .model-dropdown, .window-controls"
      )
    );
  }

  async function handleTitlebarMouseDown(event: MouseEvent) {
    if (event.button !== 0) return;
    if (isInteractiveTitlebarTarget(event.target)) return;

    await appWindow.startDragging();
  }

  function toggleModelDropdown() {
    showModelDropdown = !showModelDropdown;
  }

  function selectModel(modelValue: string) {
    showModelDropdown = false;
    onSelectModel?.(modelValue);
  }

  function toggleTheme() {
    themeStore.toggle();
  }

  const displayModelName = $derived(
    availableModels.find((m) => m.value === selectedModelValue)?.label
    ?? selectedModelValue
    ?? "选择模型"
  );

  // Close dropdown when clicking outside
  $effect(() => {
    if (!showModelDropdown) return;
    const handler = (event: MouseEvent) => {
      const target = event.target as HTMLElement;
      if (!target.closest(".model-selector")) {
        showModelDropdown = false;
      }
    };
    document.addEventListener("click", handler);
    return () => document.removeEventListener("click", handler);
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<header class="titlebar" onmousedown={handleTitlebarMouseDown}>
  <div class="titlebar-side titlebar-side-left">
    {#if isMac}
      <div class="traffic-lights">
        <button class="window-control traffic-light close" onclick={close} aria-label="Close"></button>
        <button class="window-control traffic-light minimize" onclick={minimize} aria-label="Minimize"></button>
        <button class="window-control traffic-light maximize" onclick={maximize} aria-label="Maximize"></button>
      </div>
    {/if}

    <button class="sidebar-toggle" onclick={onToggleLeft} aria-label={leftSidebarCollapsed ? "展开左侧边栏" : "收起左侧边栏"}>
      {#if leftSidebarCollapsed}
        <ChevronRight size={16} strokeWidth={2.25} />
      {:else}
        <ChevronLeft size={16} strokeWidth={2.25} />
      {/if}
    </button>

    <div class="model-selector">
      <button class="model-badge" onclick={toggleModelDropdown}>
        {displayModelName}
        <ChevronDown size={13} strokeWidth={2} />
      </button>
      {#if showModelDropdown}
        <div class="model-dropdown">
          <div class="dropdown-header">选择模型</div>
          {#if availableModels.length > 0}
            <div class="dropdown-scroll">
              {#each availableModels as model}
                <button
                  class="dropdown-item {model.value === selectedModelValue ? 'active' : ''}"
                  onclick={() => selectModel(model.value)}
                >
                  <span>{model.label}</span>
                  {#if model.value === selectedModelValue}
                    <Check size={14} strokeWidth={2} />
                  {/if}
                </button>
              {/each}
            </div>
            <div class="dropdown-divider"></div>
            <button class="dropdown-item settings-item" onclick={() => { showModelDropdown = false; onOpenSettings?.(); }}>
              <Settings size={14} strokeWidth={2} />
              <span>配置模型</span>
            </button>
          {:else}
            <div class="dropdown-empty-hint">
              <span>暂无可用模型</span>
              <button class="dropdown-item settings-item" onclick={() => { showModelDropdown = false; onOpenSettings?.(); }}>
                <Settings size={14} strokeWidth={2} />
                <span>去配置</span>
              </button>
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <div class="titlebar-drag-fill" aria-hidden="true"></div>
  </div>

  <div class="titlebar-center">
    {#if session}
      <div class="session-titlebar">
        {#if !session.title_pending && session.title_emoji}
          <span class="session-emoji" aria-hidden="true">{session.title_emoji}</span>
        {/if}
        {#if session.title_pending}
          <span class="title-skeleton" aria-hidden="true"></span>
        {:else}
          <span class="app-title">{session.title}</span>
        {/if}
      </div>
    {:else}
      <span class="app-title">{title}</span>
    {/if}
  </div>

  <div class="titlebar-side titlebar-side-right">
    <div class="titlebar-drag-fill" aria-hidden="true"></div>

    <button
      class="titlebar-icon-button"
      onclick={toggleTheme}
      aria-label={darkMode ? "切换到亮色模式" : "切换到暗色模式"}
    >
      {#if darkMode}
        <Moon size={15} strokeWidth={2} />
      {:else}
        <Sun size={15} strokeWidth={2} />
      {/if}
    </button>

    <button class="sidebar-toggle" onclick={onToggleRight} aria-label={rightSidebarCollapsed ? "展开右侧边栏" : "收起右侧边栏"}>
      {#if rightSidebarCollapsed}
        <ChevronLeft size={16} strokeWidth={2.25} />
      {:else}
        <ChevronRight size={16} strokeWidth={2.25} />
      {/if}
    </button>

    {#if !isMac}
      <div class="window-controls">
        <button class="window-control window-btn-icon" onclick={minimize} aria-label="Minimize">
          <Minus size={14} strokeWidth={2.25} />
        </button>
        <button class="window-control window-btn-icon" onclick={maximize} aria-label="Maximize">
          <Square size={12} strokeWidth={2.25} />
        </button>
        <button class="window-control window-btn-icon close-btn" onclick={close} aria-label="Close">
          <X size={14} strokeWidth={2.25} />
        </button>
      </div>
    {/if}
  </div>
</header>

<style>
  .titlebar {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
    align-items: center;
    height: 42px;
    padding: 0 14px;
    background: var(--bg-sidebar);
    border-bottom: 1px solid var(--border-default);
    user-select: none;
    -webkit-user-select: none;
  }

  .titlebar-side {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .titlebar-side-right {
    justify-content: flex-end;
  }

  .titlebar-center {
    display: flex;
    justify-content: center;
    padding: 0 16px;
  }

  .titlebar-drag-fill {
    flex: 1 1 auto;
    min-width: 16px;
    height: 100%;
  }

  .session-titlebar {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    max-width: 320px;
  }

  .app-title {
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .session-emoji {
    font-size: 14px;
    line-height: 1;
    font-family: var(--font-emoji);
  }

  .title-skeleton {
    display: inline-flex;
    width: 104px;
    height: 12px;
    border-radius: 999px;
    background:
      linear-gradient(90deg, rgba(255, 255, 255, 0) 0%, rgba(255, 255, 255, 0.52) 48%, rgba(255, 255, 255, 0) 100%),
      var(--bg-elevated);
    background-size: 180px 100%, auto;
    animation: title-shimmer 1.25s linear infinite;
  }

  .traffic-lights,
  .window-controls {
    display: flex;
    align-items: center;
  }

  .traffic-lights {
    gap: 8px;
  }

  .sidebar-toggle {
    min-width: 30px;
    height: 30px;
    padding: 0 10px;
    border: none;
    border-radius: 9px;
    background: transparent;
    color: var(--text-secondary);
    font-size: 14px;
    font-weight: 600;
    transition: background 0.15s ease, color 0.15s ease;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .titlebar-icon-button {
    min-width: 30px;
    height: 30px;
    padding: 0 10px;
    border: none;
    border-radius: 9px;
    background: transparent;
    color: var(--text-secondary);
    font-size: 14px;
    font-weight: 600;
    transition: background 0.15s ease, color 0.15s ease;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .titlebar-icon-button {
    padding: 0 8px;
  }

  .titlebar-icon-button:hover,
  .sidebar-toggle:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .traffic-light {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: none;
    cursor: pointer;
    position: relative;
    transition: opacity 0.15s ease;
  }

  .traffic-light::before {
    content: "";
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    opacity: 0;
    transition: opacity 0.15s ease;
  }

  .traffic-light.close {
    background: #ff5f57;
  }

  .traffic-light.close::before {
    width: 8px;
    height: 8px;
    background: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 8 8'%3E%3Cpath d='M1.5 1.5L6.5 6.5M1.5 6.5L6.5 1.5' stroke='%234d0000' stroke-width='1.2' fill='none'/%3E%3C/svg%3E") center no-repeat;
  }

  .traffic-light.minimize {
    background: #ffbd2e;
  }

  .traffic-light.minimize::before {
    width: 8px;
    height: 2px;
    background: #995700;
    border-radius: 1px;
  }

  .traffic-light.maximize {
    background: #28c840;
  }

  .traffic-light.maximize::before {
    width: 6px;
    height: 6px;
    background: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 6 6'%3E%3Cpath d='M1 3.5L2.5 5L5 2.5' stroke='%23006500' stroke-width='1.2' fill='none'/%3E%3C/svg%3E") center no-repeat;
  }

  .titlebar:hover .traffic-light::before {
    opacity: 1;
  }

  .window-btn-icon {
    width: 46px;
    height: 48px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    color: var(--text-secondary);
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease;
  }

  .window-btn-icon:hover {
    background: var(--bg-hover);
  }

  .close-btn:hover {
    background: #e81123;
    color: white;
  }

  @keyframes title-shimmer {
    from {
      background-position: -180px 0, 0 0;
    }

    to {
      background-position: 180px 0, 0 0;
    }
  }

  .model-selector {
    position: relative;
  }

  .model-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 10px;
    border-radius: 8px;
    border: 1px solid var(--border-default);
    background: var(--bg-surface);
    color: var(--text-primary);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
    white-space: nowrap;
    max-width: 180px;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
  }

  .model-badge::-webkit-scrollbar {
    display: none;
  }

  .model-badge:hover {
    background: var(--bg-hover);
    transform: translateY(-1px);
  }

  .model-dropdown {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    z-index: 100;
    min-width: 200px;
    max-width: 280px;
    padding: 6px;
    border-radius: 14px;
    border: 1px solid var(--border-default);
    background: var(--bg-surface);
    box-shadow: var(--shadow-dropdown);
  }

  .dropdown-header {
    padding: 8px 10px 6px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .dropdown-scroll {
    max-height: 260px;
    overflow-y: auto;
    overscroll-behavior: contain;
  }

  .dropdown-scroll::-webkit-scrollbar {
    width: 4px;
  }

  .dropdown-scroll::-webkit-scrollbar-track {
    background: transparent;
  }

  .dropdown-scroll::-webkit-scrollbar-thumb {
    background: var(--border-default);
    border-radius: 4px;
  }

  .dropdown-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    padding: 8px 10px;
    border: none;
    border-radius: 10px;
    background: transparent;
    color: var(--text-primary);
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    text-align: left;
    cursor: pointer;
    transition: background 0.15s ease;
  }

  .dropdown-item:hover {
    background: var(--bg-hover);
  }

  .dropdown-item.active {
    background: var(--bg-elevated);
    font-weight: 600;
  }

  .dropdown-divider {
    height: 1px;
    margin: 4px 10px;
    background: var(--border-default);
  }

  .dropdown-empty-hint {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px 6px;
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .settings-item {
    justify-content: flex-start;
    gap: 8px;
    color: var(--accent-primary);
    font-weight: 500;
  }

  .settings-item:hover {
    background: color-mix(in srgb, var(--accent-primary) 12%, transparent);
    color: var(--accent-primary);
  }
</style>
