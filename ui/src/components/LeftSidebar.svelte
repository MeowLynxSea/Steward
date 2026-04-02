<script lang="ts">
  import { MessageSquarePlus, Search, Settings, SlidersHorizontal, Sparkles } from "lucide-svelte";
  import type { SessionSummary } from "../lib/types";

  interface Props {
    sessions: SessionSummary[];
    activeId: string;
    collapsed?: boolean;
    onSelect: (id: string) => void;
    onCreate: () => void;
    onSettings: () => void;
  }

  let { sessions, activeId, collapsed = false, onSelect, onCreate, onSettings }: Props = $props();
</script>

<aside class="left-sidebar {collapsed ? 'collapsed' : ''}">
  {#if collapsed}
    <div class="collapsed-content">
      <button class="collapsed-icon {activeId ? 'active' : ''}" onclick={onCreate} aria-label="新会话">
        <MessageSquarePlus size={16} strokeWidth={2} />
      </button>

      {#each sessions.slice(0, 5) as session}
        <button
          class="collapsed-icon {session.id === activeId ? 'active' : ''}"
          onclick={() => onSelect(session.id)}
          title={session.title}
        >
          <Sparkles size={16} strokeWidth={2} />
        </button>
      {/each}

      <div class="collapsed-spacer"></div>
      <button class="collapsed-icon" onclick={onSettings} aria-label="设置">
        <Settings size={16} strokeWidth={2} />
      </button>
    </div>
  {:else}
    <div class="sidebar-heading">
      <span class="heading-label">会话</span>
    </div>

    <button class="new-chat-btn" onclick={onCreate}>
      <MessageSquarePlus size={16} strokeWidth={2} />
      <span>新会话</span>
    </button>

    <div class="toolbar">
      <button class="btn btn-icon btn-ghost" aria-label="搜索">
        <Search size={16} strokeWidth={2} />
      </button>
      <button class="btn btn-icon btn-ghost" aria-label="筛选">
        <SlidersHorizontal size={16} strokeWidth={2} />
      </button>
    </div>

    <div class="session-list">
      <div class="section-title">今天</div>
      {#each sessions as session}
        <button
          class="session-item {session.id === activeId ? 'active' : ''}"
          onclick={() => onSelect(session.id)}
        >
          <span class="session-icon"><Sparkles size={15} strokeWidth={2} /></span>
          <span class="session-name">{session.title}</span>
        </button>
      {/each}
    </div>

    <div class="bottom-actions">
      <button class="btn btn-ghost settings-btn" onclick={onSettings}>
        <Settings size={16} strokeWidth={2} />
        <span>设置</span>
      </button>
    </div>
  {/if}
</aside>

<style>
  .left-sidebar {
    width: 260px;
    background: #faf8f5;
    border-right: 1px solid rgba(0, 0, 0, 0.06);
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
    gap: 8px;
    height: 100%;
    padding-top: 4px;
  }

  .collapsed-icon {
    width: 36px;
    height: 36px;
    border-radius: 10px;
    background: transparent;
    border: none;
    color: #5c5c5c;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 14px;
    transition: background 0.15s ease, color 0.15s ease;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .collapsed-icon:hover {
    background: rgba(0, 0, 0, 0.05);
  }

  .collapsed-icon.active {
    background: #e8e4dc;
    color: #3d3d3d;
  }

  .collapsed-spacer {
    flex: 1;
  }

  .sidebar-heading {
    display: flex;
    align-items: center;
    padding: 4px 0 16px;
  }

  .heading-label,
  .section-title {
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: rgba(61, 61, 61, 0.56);
  }

  .new-chat-btn {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 16px;
    border-radius: 12px;
    background: transparent;
    border: 1px solid rgba(0, 0, 0, 0.1);
    color: #3d3d3d;
    font-size: 14px;
    font-weight: 500;
    width: 100%;
    margin-bottom: 16px;
    cursor: pointer;
    transition: background 0.15s ease, border-color 0.15s ease;
  }

  .new-chat-btn:hover {
    background: rgba(0, 0, 0, 0.03);
    border-color: rgba(0, 0, 0, 0.15);
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-bottom: 16px;
    border-bottom: 1px solid rgba(0, 0, 0, 0.06);
    margin-bottom: 16px;
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
    transition: background 0.15s ease, transform 0.15s ease;
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

  .session-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
    overflow-y: auto;
  }

  .section-title {
    padding: 8px 10px;
  }

  .session-item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border-radius: 10px;
    background: transparent;
    border: none;
    color: #5c5c5c;
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease;
    text-align: left;
    width: 100%;
  }

  .session-item:hover {
    background: rgba(0, 0, 0, 0.04);
  }

  .session-item.active {
    background: #e8e4dc;
    color: #3d3d3d;
  }

  .session-icon {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .session-name {
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .bottom-actions {
    padding-top: 16px;
    border-top: 1px solid rgba(0, 0, 0, 0.06);
    margin-top: 16px;
  }

  .settings-btn {
    width: 100%;
    justify-content: flex-start;
  }
</style>
