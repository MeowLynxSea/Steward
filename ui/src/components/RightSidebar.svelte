<script lang="ts">
  import {
    AlertTriangle,
    ArrowLeft,
    Check,
    ChevronDown,
    ChevronRight,
    FileText,
    Folder,
    FolderPlus,
    GitBranch,
    HardDrive,
    RefreshCw,
    Save,
    Search,
    Undo2
  } from "lucide-svelte";
  import type {
    MountedFileDiff,
    WorkspaceEntry,
    WorkspaceMountDetail,
    WorkspaceSearchResult
  } from "../lib/types";

  type ConflictResolution = "keep_disk" | "keep_workspace" | "write_copy" | "manual_merge";
  type WorkspaceTab = "files" | "changes";

  interface Props {
    currentPath: string;
    entries: WorkspaceEntry[];
    searchResults: WorkspaceSearchResult[];
    searchQuery: string;
    selectedMount: WorkspaceMountDetail | null;
    mountDiff: MountedFileDiff[];
    collapsed?: boolean;
    onSearch: (query: string) => void;
    onRefresh: () => void;
    onNavigate: (path: string) => void;
    onOpenEntry: (entry: WorkspaceEntry) => void;
    onRequestMount: () => void;
    onKeepMount: (mountId: string, scopePath?: string, checkpointId?: string) => void;
    onRevertMount: (mountId: string, scopePath?: string, checkpointId?: string) => void;
    onCreateCheckpoint: (mountId: string, label?: string, summary?: string) => void;
    onResolveConflict: (
      mountId: string,
      path: string,
      resolution: ConflictResolution,
      renamedCopyPath?: string,
      mergedContent?: string
    ) => void;
    onUseResult: (result: WorkspaceSearchResult) => void;
  }

  let {
    currentPath,
    entries,
    searchResults,
    searchQuery,
    selectedMount,
    mountDiff,
    collapsed = false,
    onSearch,
    onRefresh,
    onNavigate,
    onOpenEntry,
    onRequestMount,
    onKeepMount,
    onRevertMount,
    onCreateCheckpoint,
    onResolveConflict,
    onUseResult
  }: Props = $props();

  let localQuery = $state("");
  let activeTab = $state<WorkspaceTab>("files");
  let expandedSections = $state<Record<string, boolean>>({ "临时空间": true });
  let mergeDrafts = $state<Record<string, string>>({});
  let copyDrafts = $state<Record<string, string>>({});

  const sortedDiffs = $derived(
    [...mountDiff].sort((left, right) => {
      const weight = (status: string) => {
        if (status === "conflicted") return 0;
        if (status === "pending_delete") return 1;
        if (status === "binary_modified") return 2;
        return 3;
      };
      return weight(left.status) - weight(right.status) || left.path.localeCompare(right.path);
    })
  );

  $effect(() => {
    localQuery = searchQuery;
    for (const diff of mountDiff) {
      if (diff.status === "conflicted" && !mergeDrafts[diff.path]) {
        mergeDrafts[diff.path] = createMergeDraft(diff);
      }
      if (diff.status === "conflicted" && !copyDrafts[diff.path]) {
        copyDrafts[diff.path] = `${diff.path}.workspace-copy`;
      }
    }
  });

  function handleSearch() {
    onSearch(localQuery);
  }

  function createMergeDraft(diff: MountedFileDiff) {
    if (!diff.base_content || !diff.remote_content || !diff.working_content) {
      return diff.working_content ?? diff.remote_content ?? "";
    }
    return [
      "<<<<<<< workspace",
      diff.working_content,
      "=======",
      diff.remote_content,
      ">>>>>>> disk",
      "||||||| base",
      diff.base_content
    ].join("\n");
  }

  function statusLabel(status: string) {
    switch (status) {
      case "conflicted": return "冲突";
      case "pending_delete": return "待删除";
      case "binary_modified": return "二进制修改";
      case "added": return "新增";
      case "modified": return "修改";
      default: return status;
    }
  }

  function entryLabel(entry: WorkspaceEntry) {
    if (entry.kind === "memory_root") return "Memory";
    if (entry.kind === "mounts_root") return "Mounts";
    return entry.name ?? entry.path;
  }

  function treeIcon(entry: WorkspaceEntry) {
    if (entry.kind === "mount") return HardDrive;
    return entry.is_directory ? Folder : FileText;
  }

  function toggleSection(name: string) {
    expandedSections[name] = !expandedSections[name];
  }
</script>

<aside class="right-sidebar {collapsed ? 'collapsed' : ''}">
  <div class="right-sidebar-inner">
    <!-- Header -->
    <div class="workspace-header">
      <span class="workspace-title">工作空间</span>
      <div class="header-actions">
        <button class="icon-btn" onclick={onRequestMount} aria-label="挂载目录">
          <FolderPlus size={15} strokeWidth={2} />
        </button>
        <button class="icon-btn" onclick={onRefresh} aria-label="刷新">
          <RefreshCw size={15} strokeWidth={2} />
        </button>
      </div>
    </div>

    <!-- Tabs -->
    <div class="tab-bar">
      <button class="tab {activeTab === 'files' ? 'active' : ''}" onclick={() => activeTab = 'files'}>
        文件
      </button>
      <button class="tab {activeTab === 'changes' ? 'active' : ''}" onclick={() => activeTab = 'changes'}>
        变更
      </button>
    </div>

    {#if activeTab === "files"}
      <!-- Search -->
      <div class="search-box">
        <Search size={14} strokeWidth={2} />
        <input
          type="text"
          placeholder="搜索文件..."
          bind:value={localQuery}
          onkeydown={(event) => event.key === "Enter" && handleSearch()}
        />
      </div>

      <!-- File tree -->
      <div class="file-tree">
        {#if searchResults.length > 0}
          {#each searchResults.slice(0, 10) as result}
            <button class="tree-item" onclick={() => onUseResult(result)}>
              <Search size={13} strokeWidth={2} />
              <span class="tree-item-name">{result.document_path}</span>
            </button>
          {/each}
        {:else}
          <!-- Group entries by sections -->
          {#each entries as entry}
            {@const Icon = treeIcon(entry)}
            {#if entry.kind === "memory_root" || entry.kind === "mounts_root"}
              <button
                class="section-header"
                onclick={() => toggleSection(entryLabel(entry))}
              >
                {#if expandedSections[entryLabel(entry)]}
                  <ChevronDown size={14} strokeWidth={2} />
                {:else}
                  <ChevronRight size={14} strokeWidth={2} />
                {/if}
                <span>{entryLabel(entry)}</span>
              </button>
            {:else}
              <button
                class="tree-item {selectedMount?.summary.mount.id === entry.path && entry.kind === 'mount' ? 'active' : ''}"
                onclick={() => onOpenEntry(entry)}
              >
                <span class="tree-item-icon {entry.is_directory ? 'folder' : ''}">
                  <Icon size={14} strokeWidth={2} />
                </span>
                <span class="tree-item-name">{entryLabel(entry)}</span>
                {#if entry.conflict_count || entry.dirty_count || entry.pending_delete_count}
                  <span class="tree-item-badges">
                    {#if entry.conflict_count}
                      <span class="badge danger">{entry.conflict_count}</span>
                    {/if}
                    {#if entry.dirty_count}
                      <span class="badge">{entry.dirty_count}</span>
                    {/if}
                  </span>
                {/if}
              </button>
            {/if}
          {/each}

          {#if entries.length === 0}
            <div class="empty-hint">这里还没有内容</div>
          {/if}
        {/if}
      </div>
    {:else}
      <!-- Changes tab -->
      <div class="changes-panel">
        {#if selectedMount}
          <div class="mount-info">
            <strong>{selectedMount.summary.mount.display_name}</strong>
            <div class="mount-stats">
              <span>{selectedMount.summary.dirty_count} 变更</span>
              <span>{selectedMount.summary.conflict_count} 冲突</span>
            </div>
          </div>

          <div class="mount-actions">
            <button
              class="action-btn"
              onclick={() => onCreateCheckpoint(
                selectedMount.summary.mount.id,
                "Manual checkpoint",
                "Created from workspace rail"
              )}
              aria-label="创建 checkpoint"
            >
              <GitBranch size={14} strokeWidth={2} />
            </button>
            <button class="action-btn primary" onclick={() => onKeepMount(selectedMount.summary.mount.id)}>
              <Check size={13} strokeWidth={2} />
              保留全部
            </button>
            <button class="action-btn" onclick={() => onRevertMount(selectedMount.summary.mount.id)}>
              <Undo2 size={13} strokeWidth={2} />
              撤销全部
            </button>
          </div>

          {#if selectedMount.checkpoints.length > 0}
            <div class="checkpoint-row">
              {#each selectedMount.checkpoints.slice(0, 4) as checkpoint}
                <button
                  class="checkpoint-pill {checkpoint.is_auto ? 'auto' : 'manual'}"
                  onclick={() => onRevertMount(selectedMount.summary.mount.id, undefined, checkpoint.id)}
                >
                  <span>{checkpoint.label ?? (checkpoint.is_auto ? "Auto" : "Checkpoint")}</span>
                  <small>{checkpoint.changed_files.length} files</small>
                </button>
              {/each}
            </div>
          {/if}

          <div class="diff-list">
            {#if sortedDiffs.length === 0}
              <div class="empty-hint">当前没有待处理变更</div>
            {:else}
              {#each sortedDiffs as diff}
                <article class="diff-card {diff.status === 'conflicted' ? 'conflicted' : ''}">
                  <div class="diff-card-head">
                    <div>
                      <strong>{diff.path}</strong>
                      <p class="diff-status">{statusLabel(diff.status)}</p>
                    </div>
                    {#if diff.status === "conflicted"}
                      <span class="conflict-chip">
                        <AlertTriangle size={12} strokeWidth={2} />
                        需要处理
                      </span>
                    {/if}
                  </div>

                  {#if diff.conflict_reason}
                    <div class="notice danger">{diff.conflict_reason}</div>
                  {/if}

                  {#if diff.status === "conflicted"}
                    <div class="conflict-actions">
                      <button class="action-btn" onclick={() => onResolveConflict(selectedMount.summary.mount.id, diff.path, "keep_disk")}>
                        保留磁盘版本
                      </button>
                      <button class="action-btn" onclick={() => onResolveConflict(selectedMount.summary.mount.id, diff.path, "keep_workspace")}>
                        保留工作区版本
                      </button>
                    </div>

                    {#if diff.is_binary}
                      <div class="binary-notice">二进制冲突无法自动 merge</div>
                    {:else}
                      <div class="conflict-columns">
                        <section>
                          <span>Base</span>
                          <pre>{diff.base_content ?? "(empty)"}</pre>
                        </section>
                        <section>
                          <span>Disk</span>
                          <pre>{diff.remote_content ?? "(empty)"}</pre>
                        </section>
                        <section>
                          <span>Workspace</span>
                          <pre>{diff.working_content ?? "(empty)"}</pre>
                        </section>
                      </div>

                      <div class="manual-merge">
                        <div class="merge-head">
                          <strong>手工合并</strong>
                          <button class="action-btn" onclick={() => (mergeDrafts[diff.path] = createMergeDraft(diff))}>
                            重置草稿
                          </button>
                        </div>
                        <textarea bind:value={mergeDrafts[diff.path]} rows="12"></textarea>
                        <button
                          class="action-btn primary"
                          onclick={() => onResolveConflict(
                            selectedMount.summary.mount.id,
                            diff.path,
                            "manual_merge",
                            undefined,
                            mergeDrafts[diff.path]
                          )}
                        >
                          <Save size={13} strokeWidth={2} />
                          保存合并结果
                        </button>
                      </div>
                    {/if}

                    <div class="copy-row">
                      <input type="text" bind:value={copyDrafts[diff.path]} placeholder="冲突副本路径..." />
                      <button class="action-btn" onclick={() => onResolveConflict(
                        selectedMount.summary.mount.id,
                        diff.path,
                        "write_copy",
                        copyDrafts[diff.path]
                      )}>
                        写为副本
                      </button>
                    </div>
                  {:else}
                    {#if diff.diff_text}
                      <pre>{diff.diff_text}</pre>
                    {:else if diff.working_content}
                      <pre>{diff.working_content}</pre>
                    {:else}
                      <div class="binary-notice">二进制或删除变更</div>
                    {/if}

                    <div class="diff-item-actions">
                      <button class="action-btn primary" onclick={() => onKeepMount(selectedMount.summary.mount.id, diff.path)}>
                        保留文件
                      </button>
                      <button class="action-btn" onclick={() => onRevertMount(selectedMount.summary.mount.id, diff.path)}>
                        撤销文件
                      </button>
                    </div>
                  {/if}
                </article>
              {/each}
            {/if}
          </div>
        {:else}
          <div class="empty-hint">选择一个挂载目录查看变更</div>
        {/if}
      </div>
    {/if}
  </div>
</aside>

<style>
  .right-sidebar {
    width: 300px;
    background: var(--bg-sidebar);
    border-left: 1px solid var(--border-default);
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    overflow: hidden;
    transition: width 0.22s ease, padding 0.22s ease, opacity 0.22s ease;
  }

  .right-sidebar.collapsed {
    width: 0;
    border-left-color: transparent;
    opacity: 0;
  }

  .right-sidebar-inner {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    min-width: 280px;
    padding: 16px;
    gap: 12px;
  }

  /* Header */
  .workspace-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .workspace-title {
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .header-actions {
    display: flex;
    gap: 6px;
  }

  .icon-btn {
    width: 30px;
    height: 30px;
    border-radius: 8px;
    background: transparent;
    border: none;
    color: var(--text-tertiary);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s ease;
  }

  .icon-btn:hover {
    background: var(--bg-hover);
  }

  /* Tabs */
  .tab-bar {
    display: flex;
    gap: 2px;
    border-bottom: 1px solid var(--border-default);
    padding-bottom: 0;
  }

  .tab {
    padding: 6px 16px;
    font-size: 13px;
    font-weight: 500;
    color: var(--text-tertiary);
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    cursor: pointer;
    transition: color 0.15s ease, border-color 0.15s ease;
    margin-bottom: -1px;
  }

  .tab:hover {
    color: var(--text-primary);
  }

  .tab.active {
    color: var(--text-primary);
    border-bottom-color: var(--accent-primary);
  }

  /* Search */
  .search-box {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: 10px;
    background: var(--bg-input);
    color: var(--text-tertiary);
  }

  .search-box input {
    flex: 1;
    background: transparent;
    border: none;
    color: var(--text-primary);
    font-size: 13px;
    outline: none;
    padding: 0;
  }

  .search-box input::placeholder {
    color: var(--text-muted);
  }

  /* File tree */
  .file-tree {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .section-header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 8px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-secondary);
    background: transparent;
    border: none;
    cursor: pointer;
    width: 100%;
    text-align: left;
    transition: background 0.15s ease;
  }

  .section-header:hover {
    background: var(--bg-hover);
    border-radius: 8px;
  }

  .tree-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px 6px 24px;
    font-size: 13px;
    color: var(--text-primary);
    background: transparent;
    border: none;
    cursor: pointer;
    width: 100%;
    text-align: left;
    border-radius: 8px;
    transition: background 0.15s ease;
  }

  .tree-item:hover {
    background: var(--bg-hover);
  }

  .tree-item.active {
    background: var(--bg-active);
  }

  .tree-item-icon {
    display: inline-flex;
    align-items: center;
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .tree-item-icon.folder {
    color: var(--accent-gold);
  }

  .tree-item-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tree-item-badges {
    display: flex;
    gap: 4px;
  }

  .badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 18px;
    height: 16px;
    padding: 0 5px;
    border-radius: 999px;
    background: var(--bg-badge);
    color: var(--text-secondary);
    font-size: 10px;
    font-weight: 700;
  }

  .badge.danger {
    background: var(--accent-danger);
    color: var(--accent-danger-text);
  }

  .empty-hint {
    padding: 16px 8px;
    color: var(--text-muted);
    font-size: 12px;
  }

  /* Changes tab */
  .changes-panel {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-height: 0;
    overflow-y: auto;
  }

  .mount-info {
    padding: 0 4px;
  }

  .mount-info strong {
    font-size: 14px;
    color: var(--text-primary);
  }

  .mount-stats {
    display: flex;
    gap: 12px;
    margin-top: 4px;
    font-size: 12px;
    color: var(--text-tertiary);
  }

  .mount-actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  .action-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 5px 10px;
    border-radius: 8px;
    font-size: 12px;
    font-weight: 500;
    background: var(--bg-hover);
    color: var(--text-secondary);
    border: none;
    cursor: pointer;
    transition: background 0.15s ease, transform 0.1s ease;
  }

  .action-btn:hover {
    background: var(--bg-active);
    transform: translateY(-1px);
  }

  .action-btn.primary {
    background: var(--accent-primary);
    color: var(--text-on-dark);
  }

  .action-btn.primary:hover {
    opacity: 0.9;
  }

  .checkpoint-row {
    display: flex;
    gap: 6px;
    overflow-x: auto;
    padding-bottom: 2px;
  }

  .checkpoint-pill {
    display: inline-flex;
    flex-direction: column;
    gap: 2px;
    min-width: 90px;
    padding: 8px 10px;
    border-radius: 10px;
    background: rgba(0, 0, 0, 0.04);
    color: #5c5c5c;
    font-size: 12px;
    border: none;
    cursor: pointer;
    transition: background 0.15s ease;
  }

  .checkpoint-pill:hover {
    background: var(--bg-active);
  }

  .checkpoint-pill.auto {
    background: rgba(121, 123, 185, 0.1);
    color: #4a4f88;
  }

  .checkpoint-pill.manual {
    background: rgba(203, 162, 71, 0.12);
    color: #845f16;
  }

  .checkpoint-pill small {
    font-size: 10px;
    opacity: 0.7;
  }

  .diff-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    overflow-y: auto;
  }

  .diff-card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    border-radius: 12px;
    background: var(--bg-hover);
  }

  .diff-card.conflicted {
    box-shadow: inset 0 0 0 1px var(--accent-danger);
    background: var(--accent-danger);
  }

  .diff-card-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
  }

  .diff-card-head strong {
    font-size: 13px;
    color: var(--text-primary);
  }

  .diff-status {
    font-size: 12px;
    color: var(--text-tertiary);
    margin-top: 2px;
  }

  .conflict-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border-radius: 999px;
    padding: 4px 8px;
    background: var(--accent-danger);
    color: var(--accent-danger-text);
    font-size: 11px;
    font-weight: 600;
    white-space: nowrap;
  }

  .notice {
    border-radius: 10px;
    padding: 8px 10px;
    font-size: 12px;
  }

  .notice.danger {
    background: var(--accent-danger);
    color: var(--accent-danger-text);
  }

  .conflict-actions,
  .diff-item-actions,
  .copy-row,
  .merge-head {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }

  .conflict-columns {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
  }

  .conflict-columns section {
    border-radius: 10px;
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
    padding: 8px;
  }

  .conflict-columns section span {
    display: inline-block;
    margin-bottom: 6px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-tertiary);
  }

  .binary-notice {
    border-radius: 10px;
    background: var(--bg-hover);
    border: 1px solid var(--border-default);
    padding: 10px;
    font-size: 12px;
    color: var(--text-tertiary);
  }

  pre,
  textarea,
  input {
    box-sizing: border-box;
    width: 100%;
    border: 1px solid var(--border-input);
    border-radius: 10px;
    background: var(--bg-surface);
    padding: 8px 10px;
    font: inherit;
    color: var(--text-primary);
  }

  pre,
  textarea {
    overflow: auto;
    font-family: "SF Mono", "JetBrains Mono", monospace;
    font-size: 12px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
  }

  textarea {
    min-height: 180px;
    resize: vertical;
  }

  .manual-merge {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  @media (max-width: 1024px) {
    .right-sidebar {
      width: min(40vw, 300px);
    }

    .conflict-columns {
      grid-template-columns: 1fr;
    }
  }
</style>
