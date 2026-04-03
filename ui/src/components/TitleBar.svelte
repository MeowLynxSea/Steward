<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { AlignJustify, Minus, Square, X } from "lucide-svelte";

  interface Props {
    title?: string;
    leftSidebarCollapsed?: boolean;
    rightSidebarCollapsed?: boolean;
    onToggleLeft?: () => void;
    onToggleRight?: () => void;
  }

  let {
    title = "AionUi",
    leftSidebarCollapsed = false,
    rightSidebarCollapsed = false,
    onToggleLeft,
    onToggleRight
  }: Props = $props();

  const appWindow = getCurrentWindow();
  const isMac = navigator.platform.toLowerCase().includes("mac");

  async function minimize() {
    await appWindow.minimize();
  }

  async function maximize() {
    await appWindow.toggleMaximize();
  }

  async function close() {
    await appWindow.close();
  }
</script>

<header class="titlebar">
  <div class="titlebar-side titlebar-side-left">
    {#if isMac}
      <div class="traffic-lights">
        <button class="window-control traffic-light close" onclick={close} aria-label="Close"></button>
        <button class="window-control traffic-light minimize" onclick={minimize} aria-label="Minimize"></button>
        <button class="window-control traffic-light maximize" onclick={maximize} aria-label="Maximize"></button>
      </div>
    {/if}

    <button class="sidebar-toggle" onclick={onToggleLeft} aria-label={leftSidebarCollapsed ? "展开左侧边栏" : "收起左侧边栏"}>
      <AlignJustify size={16} strokeWidth={2} />
    </button>
  </div>

  <div class="titlebar-center">
    <span class="app-title">{title}</span>
  </div>

  <div class="titlebar-side titlebar-side-right">
    <button class="sidebar-toggle" onclick={onToggleRight} aria-label={rightSidebarCollapsed ? "展开右侧边栏" : "收起右侧边栏"}>
      <AlignJustify size={16} strokeWidth={2} />
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
    app-region: drag;
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

  .app-title {
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--text-primary);
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
    app-region: no-drag;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .sidebar-toggle:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .window-control {
    app-region: no-drag;
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
</style>
