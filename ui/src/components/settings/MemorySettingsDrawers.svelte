<script lang="ts">
  import { ChevronLeft, ChevronRight, CalendarDays, Search } from "lucide-svelte";
  import { fade, fly } from "svelte/transition";
  import type {
    MemoryDocument,
    WorkspaceEntry,
    WorkspaceSearchResult
  } from "../../lib/types";
  import {
    formatDailyLabel,
    formatMemoryTimestamp,
    type MemoryNavItem,
    type MemoryPanelMode
  } from "./memory";

  let {
    showMemoryDrawer,
    showDailyDrawer,
    memoryDrawerMode,
    activeMemoryItem,
    activeMemoryDocument,
    dailyEntries,
    activeDailyEntry,
    activeDailyDocument,
    memoryPanelLoading,
    dailyDocumentLoading,
    memoryError,
    memoryEmptyState,
    regressionQuery,
    regressionResults,
    regressionLoading,
    regressionHasSearched,
    regressionError,
    onCloseMemoryDrawer,
    onCloseDailyDrawer,
    onOpenDailyEntry,
    onRegressionQueryChange,
    onRunRegressionSearch,
    onOpenPath
  }: {
    showMemoryDrawer: boolean;
    showDailyDrawer: boolean;
    memoryDrawerMode: MemoryPanelMode;
    activeMemoryItem: MemoryNavItem | null;
    activeMemoryDocument: MemoryDocument | null;
    dailyEntries: WorkspaceEntry[];
    activeDailyEntry: WorkspaceEntry | null;
    activeDailyDocument: MemoryDocument | null;
    memoryPanelLoading: boolean;
    dailyDocumentLoading: boolean;
    memoryError: string | null;
    memoryEmptyState: string | null;
    regressionQuery: string;
    regressionResults: WorkspaceSearchResult[];
    regressionLoading: boolean;
    regressionHasSearched: boolean;
    regressionError: string | null;
    onCloseMemoryDrawer: () => void;
    onCloseDailyDrawer: () => void;
    onOpenDailyEntry: (entry: WorkspaceEntry) => Promise<void> | void;
    onRegressionQueryChange: (value: string) => void;
    onRunRegressionSearch: () => Promise<void> | void;
    onOpenPath: (path: string) => Promise<void> | void;
  } = $props();
</script>

{#if showMemoryDrawer}
  <div
    class="nested-backdrop"
    transition:fade={{ duration: 180 }}
    role="presentation"
    onclick={onCloseMemoryDrawer}
  ></div>
{/if}

{#if showMemoryDrawer && memoryDrawerMode === "daily"}
  <div
    class="memory-drawer"
    in:fly={{ x: -420, duration: 280, easing: (t) => 1 - Math.pow(1 - t, 3) }}
    out:fly={{ x: -420, duration: 220, easing: (t) => t * t }}
  >
    <div class="drawer-header">
      <button class="back-btn" onclick={onCloseMemoryDrawer} aria-label="返回">
        <ChevronLeft size={18} strokeWidth={2} />
      </button>
      <div class="header-title">
        <p class="header-eyebrow">Memory</p>
        <h3>{activeMemoryItem?.title ?? "记忆详情"}</h3>
      </div>
    </div>

    <div class="drawer-content">
      <div class="drawer-intro">
        <p>{activeMemoryItem?.description}</p>
      </div>

      {#if memoryError}
        <p class="memory-status error">{memoryError}</p>
      {:else if memoryPanelLoading}
        <p class="memory-status">正在加载 daily 日志…</p>
      {:else if dailyEntries.length === 0}
        <p class="memory-status">还没有 daily 日志。</p>
      {:else}
        <div class="drawer-scroll">
          <div class="memory-list" role="list">
            {#each dailyEntries as entry (entry.path)}
              <button class="memory-row" type="button" onclick={() => void onOpenDailyEntry(entry)}>
                <div class="memory-row-icon">
                  <CalendarDays size={15} strokeWidth={2} />
                </div>
                <div class="memory-row-main">
                  <div class="memory-row-head">
                    <strong>{formatDailyLabel(entry.path)}</strong>
                    <span>{formatMemoryTimestamp(entry.updated_at)}</span>
                  </div>
                  <p class="memory-row-path">{entry.path}</p>
                </div>
                <div class="memory-row-tail">
                  <ChevronRight size={14} strokeWidth={2} />
                </div>
              </button>
            {/each}
          </div>
        </div>
      {/if}
    </div>
  </div>
{/if}

{#if showMemoryDrawer && memoryDrawerMode === "document"}
  <div
    class="memory-drawer"
    in:fly={{ x: -420, duration: 280, easing: (t) => 1 - Math.pow(1 - t, 3) }}
    out:fly={{ x: -420, duration: 220, easing: (t) => t * t }}
  >
    <div class="drawer-header">
      <button class="back-btn" onclick={onCloseMemoryDrawer} aria-label="返回">
        <ChevronLeft size={18} strokeWidth={2} />
      </button>
      <div class="header-title">
        <p class="header-eyebrow">Memory</p>
        <h3>{activeMemoryItem?.title ?? "记忆详情"}</h3>
      </div>
    </div>

    <div class="drawer-content">
      {#if memoryError}
        <p class="memory-status error">{memoryError}</p>
      {:else if memoryPanelLoading}
        <p class="memory-status">正在加载记忆文档…</p>
      {:else if memoryEmptyState}
        <div class="empty-state">
          <p class="empty-state-title">尚未生成</p>
          <p>{memoryEmptyState}</p>
        </div>
      {:else if activeMemoryDocument}
        <section class="document-view">
          <div class="drawer-intro">
            <p>{activeMemoryItem?.description}</p>
          </div>
          <div class="document-meta">
            <span>{activeMemoryDocument.path}</span>
            <span>{formatMemoryTimestamp(activeMemoryDocument.updated_at)}</span>
            <span>{activeMemoryDocument.word_count} words</span>
          </div>
          <div class="document-scroll">
            <pre>{activeMemoryDocument.content}</pre>
          </div>
        </section>
      {/if}
    </div>
  </div>
{/if}

{#if showMemoryDrawer && memoryDrawerMode === "regression"}
  <div
    class="memory-drawer"
    in:fly={{ x: -420, duration: 280, easing: (t) => 1 - Math.pow(1 - t, 3) }}
    out:fly={{ x: -420, duration: 220, easing: (t) => t * t }}
  >
    <div class="drawer-header">
      <button class="back-btn" onclick={onCloseMemoryDrawer} aria-label="返回">
        <ChevronLeft size={18} strokeWidth={2} />
      </button>
      <div class="header-title">
        <p class="header-eyebrow">Regression</p>
        <h3>{activeMemoryItem?.title ?? "回归搜索"}</h3>
      </div>
    </div>

    <div class="drawer-content">
      <div class="drawer-intro">
        <p>{activeMemoryItem?.description}</p>
      </div>

      <div class="memory-search-row">
        <label class="memory-search-field">
          <Search size={14} strokeWidth={2} />
          <input
            type="text"
            value={regressionQuery}
            placeholder="搜索长期记忆、daily 日志或 workspace:// 挂载内容"
            oninput={(event) =>
              onRegressionQueryChange((event.currentTarget as HTMLInputElement).value)}
            onkeydown={(event) => event.key === "Enter" && void onRunRegressionSearch()}
          />
        </label>
        <button
          class="inline-tool-btn primary"
          type="button"
          onclick={() => void onRunRegressionSearch()}
          disabled={regressionLoading}
        >
          <span>{regressionLoading ? "搜索中" : "搜索"}</span>
        </button>
      </div>

      {#if regressionError}
        <p class="memory-status error">{regressionError}</p>
      {:else if regressionLoading}
        <p class="memory-status">正在执行回归搜索…</p>
      {:else if regressionHasSearched && regressionResults.length === 0}
        <p class="memory-status">没有命中结果。</p>
      {:else if regressionResults.length > 0}
        <div class="drawer-scroll">
          <div class="memory-list" role="list">
            {#each regressionResults as result (result.chunk_id)}
              <button class="memory-row" type="button" onclick={() => void onOpenPath(result.document_path)}>
                <div class="memory-row-icon">
                  <Search size={14} strokeWidth={2} />
                </div>
                <div class="memory-row-main">
                  <div class="memory-row-head">
                    <strong>{result.document_path}</strong>
                    <span>{result.score.toFixed(2)}</span>
                  </div>
                  <p>{result.content}</p>
                </div>
                <div class="memory-row-tail">
                  <ChevronRight size={14} strokeWidth={2} />
                </div>
              </button>
            {/each}
          </div>
        </div>
      {/if}
    </div>
  </div>
{/if}

{#if showDailyDrawer}
  <div
    class="tertiary-backdrop"
    transition:fade={{ duration: 160 }}
    role="presentation"
    onclick={onCloseDailyDrawer}
  ></div>

  <div
    class="daily-drawer"
    in:fly={{ x: -420, duration: 260, easing: (t) => 1 - Math.pow(1 - t, 3) }}
    out:fly={{ x: -420, duration: 200, easing: (t) => t * t }}
  >
    <div class="drawer-header">
      <button class="back-btn" onclick={onCloseDailyDrawer} aria-label="返回">
        <ChevronLeft size={18} strokeWidth={2} />
      </button>
      <div class="header-title">
        <p class="header-eyebrow">Daily</p>
        <h3>{activeDailyEntry ? formatDailyLabel(activeDailyEntry.path) : "日志详情"}</h3>
      </div>
    </div>

    <div class="drawer-content">
      {#if memoryError}
        <p class="memory-status error">{memoryError}</p>
      {:else if dailyDocumentLoading}
        <p class="memory-status">正在加载日志内容…</p>
      {:else if activeDailyDocument}
        <section class="document-view">
          <div class="document-meta">
            <span>{activeDailyDocument.path}</span>
            <span>{formatMemoryTimestamp(activeDailyDocument.updated_at)}</span>
            <span>{activeDailyDocument.word_count} words</span>
          </div>
          <div class="document-scroll">
            <pre>{activeDailyDocument.content}</pre>
          </div>
        </section>
      {/if}
    </div>
  </div>
{/if}

<style>
  :global(:root) {
    --settings-sidebar-width: min(420px, 100vw);
  }

  .nested-backdrop,
  .tertiary-backdrop {
    position: fixed;
    inset: 0;
    z-index: 42;
    background: rgba(0, 0, 0, 0.15);
    backdrop-filter: blur(8px);
  }

  .tertiary-backdrop {
    z-index: 44;
    background: rgba(0, 0, 0, 0.22);
  }

  .memory-drawer,
  .daily-drawer {
    position: fixed;
    top: 0;
    bottom: 0;
    width: var(--settings-sidebar-width);
    display: flex;
    flex-direction: column;
    background: var(--bg-surface);
    border-right: 1px solid var(--border-default);
    box-shadow: var(--shadow-dropdown);
    overflow: hidden;
  }

  .memory-drawer {
    left: 0;
    z-index: 43;
  }

  .daily-drawer {
    left: 0;
    z-index: 45;
  }

  .drawer-header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 18px 16px 14px;
    border-bottom: 1px solid var(--border-default);
  }

  .back-btn {
    width: 36px;
    height: 36px;
    border-radius: 10px;
    border: none;
    background: var(--bg-hover);
    color: var(--text-secondary);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    flex-shrink: 0;
  }

  .header-title {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .header-eyebrow {
    margin: 0;
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

  .drawer-content {
    flex: 1;
    min-height: 0;
    padding: 18px 16px;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .memory-search-row {
    display: flex;
    gap: 10px;
    align-items: stretch;
  }

  .memory-search-field {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 40px;
    padding: 0 12px;
    border-radius: 12px;
    border: 1px solid var(--border-input);
    background: var(--bg-input);
    color: var(--text-secondary);
  }

  .memory-search-field input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--text-primary);
    font: inherit;
    font-size: 13px;
  }

  .inline-tool-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 40px;
    padding: 0 14px;
    border-radius: 12px;
    border: 1px solid var(--border-input);
    background: var(--bg-input);
    color: var(--text-secondary);
    font: inherit;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
  }

  .inline-tool-btn.primary {
    border-color: color-mix(in srgb, var(--accent-primary) 30%, var(--border-input));
    color: var(--accent-primary);
  }

  .drawer-intro p,
  .memory-status {
    margin: 0;
    font-size: 12px;
    line-height: 1.55;
    color: var(--text-secondary);
  }

  .memory-status.error {
    color: var(--accent-danger, #c8594f);
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 4px;
  }

  .empty-state-title {
    margin: 0;
    font-size: 14px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .empty-state p {
    margin: 0;
    font-size: 13px;
    line-height: 1.6;
    color: var(--text-secondary);
  }

  .drawer-scroll,
  .document-scroll {
    min-height: 0;
    flex: 1;
    overflow-y: auto;
  }

  .memory-list {
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border-subtle, var(--border-default));
  }

  .memory-row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 13px 0;
    border: none;
    border-bottom: 1px solid var(--border-subtle, var(--border-default));
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .memory-row-icon {
    width: 28px;
    height: 28px;
    border-radius: 10px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--text-tertiary);
    background: color-mix(in srgb, var(--bg-input) 88%, transparent);
    flex-shrink: 0;
  }

  .memory-row-main {
    min-width: 0;
    flex: 1;
  }

  .memory-row-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }

  .memory-row-head strong {
    min-width: 0;
    font-size: 13px;
    font-weight: 650;
    color: var(--text-primary);
  }

  .memory-row-head span,
  .document-meta {
    font-size: 11px;
    color: var(--text-muted);
  }

  .memory-row-main p {
    margin: 4px 0 0;
    font-size: 12px;
    line-height: 1.55;
    color: var(--text-secondary);
    word-break: break-word;
  }

  .memory-row-tail {
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .document-view {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-height: 0;
    flex: 1;
  }

  .document-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 12px;
  }

  .document-scroll {
    padding-top: 14px;
    border-top: 1px solid var(--border-subtle, var(--border-default));
  }

  .document-scroll pre {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font: inherit;
    font-size: 13px;
    line-height: 1.7;
    color: var(--text-primary);
  }

  @media (max-width: 1260px) {
    .daily-drawer {
      left: 0;
      width: 100vw;
    }
  }

  @media (max-width: 840px) {
    .memory-drawer,
    .daily-drawer {
      left: 0;
      width: 100vw;
    }
  }

  @media (max-width: 640px) {
    .memory-search-row {
      flex-direction: column;
    }
  }
</style>
