<script lang="ts">
  import { List, LoaderCircle, Plus, Search, Settings, Trash2, Zap } from "lucide-svelte";
  import type { SessionSummary } from "../lib/types";

  interface Props {
    sessions: SessionSummary[];
    activeId: string;
    collapsed?: boolean;
    onSelect: (id: string) => void;
    onCreate: () => void;
    onDelete: (id: string) => void;
    onSettings: () => void;
  }

  let { sessions, activeId, collapsed = false, onSelect, onCreate, onDelete, onSettings }: Props = $props();
</script>

<aside class="left-sidebar {collapsed ? 'collapsed' : ''}">
  {#if collapsed}
    <div class="collapsed-content">
      <button class="collapsed-icon" onclick={onCreate} aria-label="新会话">
        <Plus size={16} strokeWidth={2} />
      </button>

      {#each sessions.slice(0, 5) as session}
        <button
          class="collapsed-icon {session.id === activeId ? 'active' : ''}"
          onclick={() => onSelect(session.id)}
          title={session.title}
        >
          {#if session.title_pending}
            <span class="loading-icon"><LoaderCircle size={14} strokeWidth={2} /></span>
          {:else if session.title_emoji}
            <span class="collapsed-emoji" aria-hidden="true">{session.title_emoji}</span>
          {:else}
            <Zap size={14} strokeWidth={2} />
          {/if}
        </button>
      {/each}

      <div class="collapsed-spacer"></div>
      <button class="collapsed-icon" onclick={onSettings} aria-label="设置">
        <Settings size={16} strokeWidth={2} />
      </button>
    </div>
  {:else}
    <div class="action-row">
      <button class="new-chat-btn" onclick={onCreate}>
        <Plus size={16} strokeWidth={2} />
        <span>新会话</span>
      </button>
      <button class="toolbar-icon" aria-label="搜索">
        <Search size={16} strokeWidth={2} />
      </button>
      <button class="toolbar-icon" aria-label="列表">
        <List size={16} strokeWidth={2} />
      </button>
    </div>

    <div class="session-list">
      <div class="section-title">所有会话</div>
      {#each sessions as session}
        <div class="session-row">
          <button
            class="session-item {session.id === activeId ? 'active' : ''}"
            onclick={() => onSelect(session.id)}
          >
            <span class="session-icon">
              {#if session.title_pending}
                <span class="loading-icon"><LoaderCircle size={14} strokeWidth={2} /></span>
              {:else if session.title_emoji}
                <span class="session-emoji" aria-hidden="true">{session.title_emoji}</span>
              {:else}
                <Zap size={14} strokeWidth={2} />
              {/if}
            </span>
            {#if session.title_pending}
              <span class="session-name-skeleton" aria-hidden="true"></span>
            {:else}
              <span class="session-name">{session.title}</span>
            {/if}
          </button>
          <button
            class="session-delete"
            onclick={(e) => { e.stopPropagation(); onDelete(session.id); }}
            aria-label="删除会话"
          >
            <Trash2 size={13} strokeWidth={2} />
          </button>
        </div>
      {/each}
    </div>

    <div class="bottom-actions">
      <button class="settings-btn" onclick={onSettings}>
        <Settings size={16} strokeWidth={2} />
        <span>设置</span>
      </button>
    </div>
  {/if}
</aside>

<style>
  .left-sidebar {
    width: 240px;
    background: var(--bg-sidebar);
    border-right: 1px solid var(--border-default);
    display: flex;
    flex-direction: column;
    padding: 16px;
    height: 100%;
    transition: width 0.2s ease, padding 0.2s ease;
  }

  .left-sidebar.collapsed {
    width: 56px;
    padding: 16px 8px;
  }

  .collapsed-content {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    height: 100%;
    padding-top: 4px;
  }

  .collapsed-icon {
    width: 36px;
    height: 36px;
    border-radius: 10px;
    background: transparent;
    border: none;
    color: var(--text-secondary);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s ease, color 0.15s ease;
  }

  .collapsed-icon:hover {
    background: var(--bg-hover);
  }

  .collapsed-icon.active {
    background: var(--bg-active);
    color: var(--text-primary);
  }

  .collapsed-spacer {
    flex: 1;
  }

  .collapsed-emoji {
    font-size: 14px;
    line-height: 1;
  }

  .action-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 20px;
  }

  .new-chat-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    border-radius: 10px;
    background: transparent;
    border: 1px solid var(--border-input);
    color: var(--text-primary);
    font-size: 14px;
    font-weight: 500;
    flex: 1;
    cursor: pointer;
    transition: background 0.15s ease, border-color 0.15s ease;
  }

  .new-chat-btn:hover {
    background: var(--bg-hover);
    border-color: var(--border-default);
  }

  .toolbar-icon {
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
    flex-shrink: 0;
    transition: background 0.15s ease;
  }

  .toolbar-icon:hover {
    background: var(--bg-hover);
  }

  .session-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    overflow-y: auto;
  }

  .section-title {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-muted);
    padding: 8px 10px 6px;
  }

  .session-row {
    display: flex;
    align-items: center;
    border-radius: 10px;
    position: relative;
  }

  .session-row:hover .session-delete {
    opacity: 1;
  }

  .session-item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 9px 12px;
    border-radius: 10px;
    background: transparent;
    border: none;
    color: var(--text-secondary);
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease;
    text-align: left;
    width: 100%;
  }

  .session-item:hover {
    background: var(--bg-hover);
  }

  .session-item.active {
    background: var(--bg-active);
    color: var(--text-primary);
  }

  .session-delete {
    position: absolute;
    right: 6px;
    width: 28px;
    height: 28px;
    border-radius: 8px;
    background: transparent;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    opacity: 0;
    transition: opacity 0.15s ease, background 0.15s ease, color 0.15s ease;
    flex-shrink: 0;
  }

  .session-delete:hover {
    background: var(--accent-danger);
    color: var(--accent-danger-text);
  }

  .session-icon {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--accent-gold);
    width: 18px;
  }

  .session-name {
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .session-emoji {
    font-size: 14px;
    line-height: 1;
    font-family: var(--font-emoji);
  }

  .collapsed-emoji {
    font-size: 13px;
    line-height: 1;
    font-family: var(--font-emoji);
  }

  .loading-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    animation: session-spin 0.85s linear infinite;
  }

  .session-name-skeleton {
    display: inline-flex;
    width: 84px;
    height: 12px;
    border-radius: 999px;
    background:
      linear-gradient(90deg, rgba(255, 255, 255, 0) 0%, rgba(255, 255, 255, 0.55) 48%, rgba(255, 255, 255, 0) 100%),
      var(--bg-elevated);
    background-size: 180px 100%, auto;
    animation: title-shimmer 1.2s linear infinite;
  }

  .bottom-actions {
    padding-top: 12px;
    border-top: 1px solid var(--border-default);
    margin-top: 8px;
  }

  .settings-btn {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: 10px;
    background: transparent;
    border: none;
    color: var(--text-tertiary);
    font-size: 14px;
    font-weight: 500;
    width: 100%;
    cursor: pointer;
    transition: background 0.15s ease;
  }

  .settings-btn:hover {
    background: var(--bg-hover);
  }

  @keyframes title-shimmer {
    from {
      background-position: -180px 0, 0 0;
    }

    to {
      background-position: 180px 0, 0 0;
    }
  }

  @keyframes session-spin {
    from {
      transform: rotate(0deg);
    }

    to {
      transform: rotate(360deg);
    }
  }
</style>
