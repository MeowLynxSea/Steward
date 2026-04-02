<script lang="ts">
  import { FileText, Folder, RefreshCw, Search } from "lucide-svelte";
  import type { WorkspaceEntry, WorkspaceSearchResult } from "../lib/types";

  interface Props {
    entries: WorkspaceEntry[];
    searchResults: WorkspaceSearchResult[];
    searchQuery: string;
    collapsed?: boolean;
    onSearch: (query: string) => void;
    onRefresh: () => void;
    onUseResult: (result: WorkspaceSearchResult) => void;
  }

  let { entries, searchResults, searchQuery, collapsed = false, onSearch, onRefresh, onUseResult }: Props = $props();

  let activeTab = $state("files");
  let localQuery = $state("");

  $effect(() => {
    localQuery = searchQuery;
  });

  function handleSearch() {
    onSearch(localQuery);
  }

</script>

<aside class="right-sidebar {collapsed ? 'collapsed' : ''}">
  <div class="right-sidebar-inner">
    <div class="workspace-header">
      <span class="workspace-title">工作空间</span>
      <div class="header-actions">
        <button class="btn btn-icon btn-ghost" onclick={onRefresh} aria-label="刷新">
          <RefreshCw size={16} strokeWidth={2} />
        </button>
      </div>
    </div>

    <div class="workspace-tabs">
      <button
        class="workspace-tab {activeTab === 'files' ? 'active' : ''}"
        onclick={() => activeTab = "files"}
      >
        文件
      </button>
      <button
        class="workspace-tab {activeTab === 'changes' ? 'active' : ''}"
        onclick={() => activeTab = "changes"}
      >
        变更
      </button>
    </div>

    <div class="search-box">
      <Search size={15} strokeWidth={2} />
      <input
        type="text"
        placeholder="搜索文件..."
        bind:value={localQuery}
        onkeydown={(event) => event.key === "Enter" && handleSearch()}
      />
    </div>

    <div class="folder-tree">
      {#if searchResults.length > 0}
        {#each searchResults.slice(0, 10) as result}
          <button class="folder-item" onclick={() => onUseResult(result)}>
            <span class="folder-icon"><FileText size={15} strokeWidth={2} /></span>
            <span class="folder-name">{result.document_path}</span>
          </button>
        {/each}
      {:else if entries.length > 0}
        {#each entries as entry}
          <div class="folder-item">
            <span class="folder-icon">
              {#if entry.is_directory}
                <Folder size={15} strokeWidth={2} />
              {:else}
                <FileText size={15} strokeWidth={2} />
              {/if}
            </span>
            <span class="folder-name">{entry.path}</span>
          </div>
        {/each}
      {:else}
        <p class="muted">暂无文件</p>
      {/if}
    </div>
  </div>
</aside>

<style>
  .right-sidebar {
    width: 280px;
    background: #faf8f5;
    border-left: 1px solid rgba(0, 0, 0, 0.06);
    display: flex;
    flex-direction: column;
    padding: 16px;
    height: 100%;
    min-width: 0;
    overflow: hidden;
    transition:
      width 0.22s ease,
      padding 0.22s ease,
      border-color 0.22s ease,
      opacity 0.22s ease;
  }

  .right-sidebar.collapsed {
    width: 0;
    padding: 0;
    border-left-color: transparent;
    opacity: 0;
  }

  .right-sidebar-inner {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    min-width: 248px;
    opacity: 1;
    transform: translateX(0);
    transition:
      opacity 0.16s ease,
      transform 0.22s ease;
  }

  .right-sidebar.collapsed .right-sidebar-inner {
    opacity: 0;
    transform: translateX(12px);
    pointer-events: none;
  }

  .workspace-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding-bottom: 16px;
    border-bottom: 1px solid rgba(0, 0, 0, 0.06);
    margin-bottom: 16px;
  }

  .workspace-title {
    font-size: 14px;
    font-weight: 600;
    color: #3d3d3d;
  }

  .header-actions {
    display: flex;
    gap: 4px;
  }

  .workspace-tabs {
    display: flex;
    gap: 16px;
    margin-bottom: 16px;
  }

  .workspace-tab {
    font-size: 13px;
    color: rgba(61, 61, 61, 0.6);
    padding-bottom: 8px;
    cursor: pointer;
    transition: color 0.15s ease, border-color 0.15s ease;
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
  }

  .workspace-tab:hover {
    color: #3d3d3d;
  }

  .workspace-tab.active {
    color: #3d3d3d;
    border-bottom-color: #3d3d3d;
    font-weight: 500;
  }

  .search-box {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    background: #f5f0e8;
    border-radius: 10px;
    margin-bottom: 16px;
  }

  .search-box input {
    flex: 1;
    background: transparent;
    font-size: 13px;
    color: #3d3d3d;
    border: none;
    outline: none;
  }

  .search-box input::placeholder {
    color: rgba(61, 61, 61, 0.4);
  }

  .folder-tree {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .folder-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-radius: 8px;
    font-size: 13px;
    color: #5c5c5c;
    cursor: pointer;
    background: transparent;
    border: none;
    text-align: left;
    width: 100%;
  }

  .folder-item:hover {
    background: rgba(0, 0, 0, 0.04);
  }

  .folder-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .folder-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .muted {
    color: rgba(61, 61, 61, 0.6);
    font-size: 13px;
    padding: 8px 10px;
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
</style>
