<script lang="ts">
  import {
    ChevronLeft,
    ChevronRight,
    History,
    Search
  } from "lucide-svelte";
  import { fade, fly } from "svelte/transition";
  import type {
    MemoryNodeDetail,
    MemorySearchHit,
    MemoryVersion
  } from "../../lib/types";
  import {
    formatMemoryTimestamp,
    routeSegment,
    memoryKindLabel,
    routeLabel,
    type MemoryNavItem,
    type MemoryPanelMode
  } from "./memory";

  let {
    showMemoryDrawer,
    memoryDrawerMode,
    activeMemoryItem,
    selectedNode,
    selectedVersions,
    memoryPanelLoading,
    memoryError,
    memorySearchQuery,
    memorySearchResults,
    memorySearchLoading,
    memorySearchHasSearched,
    memorySearchError,
    onCloseMemoryDrawer,
    onMemorySearchQueryChange,
    onRunMemorySearch,
    onOpenMemoryKey
  }: {
    showMemoryDrawer: boolean;
    memoryDrawerMode: MemoryPanelMode;
    activeMemoryItem: MemoryNavItem | null;
    selectedNode: MemoryNodeDetail | null;
    selectedVersions: MemoryVersion[];
    memoryPanelLoading: boolean;
    memoryError: string | null;
    memorySearchQuery: string;
    memorySearchResults: MemorySearchHit[];
    memorySearchLoading: boolean;
    memorySearchHasSearched: boolean;
    memorySearchError: string | null;
    onCloseMemoryDrawer: () => void;
    onMemorySearchQueryChange: (value: string) => void;
    onRunMemorySearch: () => Promise<void> | void;
    onOpenMemoryKey: (key: string) => Promise<void> | void;
  } = $props();
</script>

{#if showMemoryDrawer}
  <div
    class="nested-backdrop"
    transition:fade={{ duration: 180 }}
    role="presentation"
    onclick={onCloseMemoryDrawer}
  ></div>

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
        <h3>{activeMemoryItem?.title ?? "Memory Inspector"}</h3>
      </div>
    </div>

    <div class="drawer-content">
      {#if memoryDrawerMode === "search"}
        <div class="drawer-intro">
          <p>{activeMemoryItem?.description}</p>
        </div>

        <div class="memory-search-row">
          <label class="memory-search-field">
            <Search size={14} strokeWidth={2} />
            <input
              type="text"
              value={memorySearchQuery}
              placeholder="搜索 routes、keywords、episodes 和 curated memories"
              oninput={(event) =>
                onMemorySearchQueryChange((event.currentTarget as HTMLInputElement).value)}
              onkeydown={(event) => event.key === "Enter" && void onRunMemorySearch()}
            />
          </label>
          <button
            class="inline-tool-btn primary"
            type="button"
            onclick={() => void onRunMemorySearch()}
            disabled={memorySearchLoading}
          >
            <span>{memorySearchLoading ? "搜索中" : "搜索"}</span>
          </button>
        </div>

        {#if memorySearchError}
          <p class="memory-status error">{memorySearchError}</p>
        {:else if memorySearchLoading}
          <p class="memory-status">正在检索 memory graph…</p>
        {:else if memorySearchHasSearched && memorySearchResults.length === 0}
          <p class="memory-status">没有命中结果。</p>
        {:else if memorySearchResults.length > 0}
          <div class="drawer-scroll">
            <div class="memory-list" role="list">
              {#each memorySearchResults as result (result.route_id)}
                <button class="memory-row" type="button" onclick={() => void onOpenMemoryKey(result.uri)}>
                  <div class="memory-row-main">
                    <div class="memory-row-head">
                      <strong>{result.title}</strong>
                      <span>{memoryKindLabel(result.kind)}</span>
                    </div>
                    <p>{result.uri}</p>
                    <p>{result.content_snippet}</p>
                  </div>
                  <div class="memory-row-tail">
                    <ChevronRight size={14} strokeWidth={2} />
                  </div>
                </button>
              {/each}
            </div>
          </div>
        {/if}
      {:else if memoryError}
        <p class="memory-status error">{memoryError}</p>
      {:else if memoryPanelLoading}
        <p class="memory-status">正在加载记忆节点…</p>
      {:else if selectedNode}
        <div class="drawer-scroll">
          <section class="document-view">
            <div class="drawer-intro">
              <p>
                {(selectedNode.selected_route ?? selectedNode.primary_route)
                  ? routeLabel(selectedNode.selected_route ?? selectedNode.primary_route)
                  : "这个节点当前没有 primary route。"}
              </p>
            </div>

            <div class="document-meta">
              <strong>
                {routeSegment(selectedNode.selected_route ?? selectedNode.primary_route) ??
                  selectedNode.node.title}
              </strong>
            </div>

            <div class="document-meta">
              <span>{memoryKindLabel(selectedNode.node.kind)}</span>
              <span>{formatMemoryTimestamp(selectedNode.node.updated_at)}</span>
              <span>{selectedNode.active_version.status}</span>
            </div>

            <div class="document-scroll">
              <pre>{selectedNode.active_version.content}</pre>
            </div>
          </section>

          {#if selectedNode.routes.length > 0}
            <section class="inspector-section">
              <div class="section-head">
                <History size={14} strokeWidth={2} />
                <h4>Routes & Aliases</h4>
              </div>
              <div class="pill-grid">
                {#each selectedNode.routes as route (route.id)}
                  <button class="pill-button" type="button" onclick={() => void onOpenMemoryKey(routeLabel(route))}>
                    {routeLabel(route)}
                  </button>
                {/each}
              </div>
            </section>
          {/if}

          {#if selectedNode.keywords.length > 0}
            <section class="inspector-section">
              <div class="section-head">
                <Search size={14} strokeWidth={2} />
                <h4>Keywords</h4>
              </div>
              <div class="pill-grid">
                {#each selectedNode.keywords as keyword (keyword.id)}
                  <span class="pill">{keyword.keyword}</span>
                {/each}
              </div>
            </section>
          {/if}

          {#if selectedNode.edges.length > 0}
            <section class="inspector-section">
              <div class="section-head">
                <History size={14} strokeWidth={2} />
                <h4>Triggers & Relations</h4>
              </div>
              <div class="detail-list">
                {#each selectedNode.edges as edge (edge.id)}
                  <div class="detail-card">
                    <strong>{edge.relation_kind}</strong>
                    <p>visibility: {edge.visibility} · priority: {edge.priority}</p>
                    {#if edge.trigger_text}
                      <p>{edge.trigger_text}</p>
                    {/if}
                  </div>
                {/each}
              </div>
            </section>
          {/if}

          {#if selectedNode.related_nodes.length > 0}
            <section class="inspector-section">
              <div class="section-head">
                <History size={14} strokeWidth={2} />
                <h4>Related Nodes</h4>
              </div>
              <div class="detail-list">
                {#each selectedNode.related_nodes as node (node.route_id)}
                  <button class="detail-card action-card" type="button" onclick={() => void onOpenMemoryKey(node.uri)}>
                    <strong>{routeSegment(node.uri) ?? node.title}</strong>
                    <p>{node.uri}</p>
                    <p>{node.content_snippet}</p>
                  </button>
                {/each}
              </div>
            </section>
          {/if}

          {#if selectedVersions.length > 0}
            <section class="inspector-section">
              <div class="section-head">
                <History size={14} strokeWidth={2} />
                <h4>Version History</h4>
              </div>
              <div class="detail-list">
                {#each selectedVersions as version (version.id)}
                  <div class="detail-card">
                    <strong>{version.status}</strong>
                    <p>{formatMemoryTimestamp(version.created_at)}</p>
                    <p>{version.content.slice(0, 180)}</p>
                  </div>
                {/each}
              </div>
            </section>
          {/if}
        </div>
      {:else}
        <p class="memory-status">未找到这个记忆节点。</p>
      {/if}
    </div>
  </div>
{/if}

<style>
  :global(:root) {
    --settings-sidebar-width: min(420px, 100vw);
  }

  .nested-backdrop {
    position: fixed;
    inset: 0;
    z-index: 42;
    background: rgba(0, 0, 0, 0.15);
    backdrop-filter: blur(8px);
  }

  .memory-drawer {
    position: fixed;
    top: 0;
    bottom: 0;
    left: 0;
    z-index: 43;
    width: var(--settings-sidebar-width);
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
    gap: 12px;
    padding: 18px 20px;
    border-bottom: 1px solid var(--border-subtle, var(--border-default));
  }

  .back-btn {
    width: 34px;
    height: 34px;
    border-radius: 12px;
    border: 1px solid var(--border-subtle, var(--border-default));
    background: var(--bg-input);
    color: var(--text-primary);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }

  .header-title {
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

  .header-title h3 {
    margin: 0;
    font-size: 17px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .drawer-content,
  .drawer-scroll {
    min-height: 0;
    flex: 1;
    display: flex;
    flex-direction: column;
  }

  .drawer-content {
    padding: 18px 20px 22px;
    gap: 14px;
  }

  .drawer-scroll {
    overflow-y: auto;
    gap: 18px;
  }

  .drawer-intro p,
  .memory-status {
    margin: 0;
    font-size: 12px;
    line-height: 1.6;
    color: var(--text-secondary);
  }

  .memory-status.error {
    color: var(--accent-danger, #c8594f);
  }

  .memory-search-row {
    display: flex;
    gap: 10px;
  }

  .memory-search-field {
    min-width: 0;
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px;
    border-radius: 14px;
    border: 1px solid var(--border-default);
    background: var(--bg-input);
    color: var(--text-secondary);
  }

  .memory-search-field input {
    width: 100%;
    height: 38px;
    border: none;
    outline: none;
    background: transparent;
    color: var(--text-primary);
    font-size: 13px;
  }

  .inline-tool-btn {
    height: 38px;
    padding: 0 14px;
    border-radius: 12px;
    border: 1px solid var(--border-default);
    background: var(--bg-input);
    color: var(--text-primary);
    display: inline-flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
  }

  .inline-tool-btn.primary {
    background: color-mix(in srgb, var(--accent-primary, #5f8cff) 14%, var(--bg-input));
  }

  .memory-list,
  .detail-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .memory-row {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 12px 0;
    border: none;
    border-bottom: 1px solid var(--border-subtle, var(--border-default));
    background: transparent;
    text-align: left;
    color: inherit;
  }

  .memory-row {
    cursor: pointer;
  }

  .memory-row-main {
    min-width: 0;
    flex: 1;
  }

  .memory-row-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .memory-row-head strong {
    font-size: 13px;
    font-weight: 650;
    color: var(--text-primary);
  }

  .memory-row-head span {
    flex-shrink: 0;
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .memory-row-main p,
  .detail-card p {
    margin: 4px 0 0;
    font-size: 12px;
    line-height: 1.55;
    color: var(--text-secondary);
    word-break: break-word;
  }

  .memory-row-tail {
    flex-shrink: 0;
  }

  .document-view {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .document-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .document-scroll pre {
    margin: 0;
    padding: 14px;
    border-radius: 14px;
    background: color-mix(in srgb, var(--bg-input) 88%, transparent);
    color: var(--text-primary);
    font-size: 12px;
    line-height: 1.65;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .inspector-section {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .section-head {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-secondary);
  }

  .section-head h4 {
    margin: 0;
    font-size: 13px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .pill-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .pill,
  .pill-button {
    padding: 7px 11px;
    border-radius: 999px;
    border: 1px solid var(--border-default);
    font-size: 11px;
    line-height: 1.3;
  }

  .pill {
    background: color-mix(in srgb, var(--bg-input) 90%, transparent);
    color: var(--text-secondary);
  }

  .pill-button,
  .action-card {
    background: transparent;
    color: var(--text-primary);
    cursor: pointer;
  }

  .detail-card {
    padding: 12px 14px;
    border-radius: 14px;
    border: 1px solid var(--border-subtle, var(--border-default));
    background: color-mix(in srgb, var(--bg-input) 90%, transparent);
  }

  .detail-card strong {
    font-size: 13px;
    font-weight: 650;
    color: var(--text-primary);
  }

  @media (max-width: 640px) {
    .memory-drawer {
      width: 100vw;
    }

    .memory-search-row {
      flex-direction: column;
    }
  }
</style>
