<script lang="ts">
  interface Props {
    sessionTitle?: string;
    modelName?: string;
  }

  let { sessionTitle, modelName = "MiniMax-M1.7" }: Props = $props();
</script>

<header class="top-nav">
  <div class="top-nav-left">
    {#if sessionTitle}
      <span class="model-badge">{modelName}</span>
      <span class="session-title">{sessionTitle}</span>
    {:else}
      <button class="menu-btn">☰</button>
    {/if}
  </div>

  <div class="top-nav-center">
    <span class="app-name">Steward</span>
  </div>

  <div class="top-nav-right">
    <div class="status-dot"></div>
    <span class="status-text">Cowork</span>
    <button class="btn btn-icon btn-ghost">⛶</button>
  </div>
</header>

<style>
  [data-theme="dark"] .top-nav {
    background: var(--bg-sidebar);
    border-bottom-color: var(--border-default);
  }

  .top-nav {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 24px;
    border-bottom: 1px solid rgba(0, 0, 0, 0.06);
    background: #faf8f5;
    height: 56px;
  }

  .top-nav-left {
    display: flex;
    align-items: center;
    gap: 16px;
    flex: 1;
  }

  .top-nav-center {
    display: flex;
    align-items: center;
    justify-content: center;
    flex: 1;
  }

  .top-nav-right {
    display: flex;
    align-items: center;
    gap: 12px;
    flex: 1;
    justify-content: flex-end;
  }

  .model-badge {
    display: inline-flex;
    align-items: center;
    padding: 6px 12px;
    border-radius: 20px;
    background: #e8e4dc;
    color: #5c5c5c;
    font-size: 13px;
    font-weight: 500;
  }

  .session-title {
    font-size: 14px;
    color: #3d3d3d;
    font-weight: 500;
  }

  .menu-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border-radius: 10px;
    background: transparent;
    border: none;
    cursor: pointer;
    font-size: 18px;
    color: #3d3d3d;
  }

  .menu-btn:hover {
    background: rgba(0, 0, 0, 0.05);
  }

  .app-name {
    font-size: 14px;
    font-weight: 600;
    color: #3d3d3d;
  }

  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #4ade80;
  }

  .status-text {
    font-size: 13px;
    color: #5c5c5c;
    font-weight: 500;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 8px 14px;
    border-radius: 10px;
    font-size: 14px;
    font-weight: 500;
    border: none;
    background: transparent;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn:hover {
    transform: translateY(-1px);
  }

  .btn-ghost {
    background: transparent;
    color: #6b6b6b;
  }

  .btn-ghost:hover {
    background: rgba(0, 0, 0, 0.05);
  }

  .btn-icon {
    width: 36px;
    height: 36px;
    padding: 0;
  }

  [data-theme="dark"] .model-badge {
    background: var(--bg-elevated);
    color: var(--text-secondary);
  }

  [data-theme="dark"] .session-title,
  [data-theme="dark"] .menu-btn,
  [data-theme="dark"] .app-name {
    color: var(--text-primary);
  }

  [data-theme="dark"] .menu-btn:hover {
    background: var(--bg-hover);
  }

  [data-theme="dark"] .status-text {
    color: var(--text-secondary);
  }

  [data-theme="dark"] .btn-ghost {
    color: var(--text-tertiary);
  }

  [data-theme="dark"] .btn-ghost:hover {
    background: var(--bg-hover);
  }
</style>