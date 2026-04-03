<script lang="ts">
  import {
    AlertTriangle,
    ArrowLeft,
    Check,
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

  const pathSegments = $derived.by(() => {
    const normalized = currentPath.replace(/\/+$/, "") || "workspace://";
    if (normalized === "workspace://" || normalized === "workspace:") {
      return [{ label: "workspace://", path: "workspace://" }];
    }

    const rest = normalized.replace(/^workspace:\/\//, "");
    const parts = rest.split("/").filter(Boolean);
    const items = [{ label: "workspace://", path: "workspace://" }];
    let prefix = "";

    for (const part of parts) {
      prefix = prefix ? `${prefix}/${part}` : part;
      items.push({
        label: part,
        path: `workspace://${prefix}`
      });
    }

    return items;
  });

  const parentPath = $derived.by(() => {
    if (pathSegments.length <= 1) return null;
    return pathSegments[pathSegments.length - 2]?.path ?? null;
  });

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
      case "conflicted":
        return "冲突";
      case "pending_delete":
        return "待删除";
      case "binary_modified":
        return "二进制修改";
      case "added":
        return "新增";
      case "modified":
        return "修改";
      default:
        return status;
    }
  }

  function entryLabel(entry: WorkspaceEntry) {
    if (entry.kind === "memory_root") return "Memory";
    if (entry.kind === "mounts_root") return "Mounts";
    return entry.name ?? entry.path;
  }

  function isRootEntry(entry: WorkspaceEntry) {
    return entry.kind === "memory_root" || entry.kind === "mounts_root";
  }

  function treeIcon(entry: WorkspaceEntry) {
    if (entry.kind === "mount") return HardDrive;
    return entry.is_directory ? Folder : FileText;
  }

</script>

<aside class="right-sidebar {collapsed ? 'collapsed' : ''}">
  <div class="right-sidebar-inner">
    <div class="workspace-header">
      <div class="title-cluster">
        <span class="workspace-title">Workspace</span>
        <div class="path-strip">
          {#each pathSegments as segment, index}
            <button class="path-segment" onclick={() => onNavigate(segment.path)}>
              {segment.label}
            </button>
            {#if index < pathSegments.length - 1}
              <ChevronRight size={13} strokeWidth={2} />
            {/if}
          {/each}
        </div>
      </div>
      <div class="header-actions">
        <button class="icon-button" onclick={onRequestMount} aria-label="挂载目录">
          <FolderPlus size={16} strokeWidth={2} />
        </button>
        <button class="icon-button" onclick={onRefresh} aria-label="刷新">
          <RefreshCw size={16} strokeWidth={2} />
        </button>
      </div>
    </div>

    <div class="search-box">
      <Search size={15} strokeWidth={2} />
      <input
        type="text"
        placeholder="搜索 workspace://..."
        bind:value={localQuery}
        onkeydown={(event) => event.key === "Enter" && handleSearch()}
      />
    </div>

    <div class="tree-shell">
      {#if parentPath}
        <button class="tree-row back-row" onclick={() => onNavigate(parentPath)}>
          <span class="tree-branch">
            <ArrowLeft size={14} strokeWidth={2} />
          </span>
          <span class="tree-name">..</span>
        </button>
      {/if}

      {#if searchResults.length > 0}
        <div class="tree-list">
          {#each searchResults.slice(0, 10) as result}
            <button class="tree-row search-row" onclick={() => onUseResult(result)}>
              <span class="tree-branch">
                <Search size={14} strokeWidth={2} />
              </span>
              <span class="tree-label">
                <strong>{result.document_path}</strong>
                <span>{result.content.slice(0, 90)}</span>
              </span>
            </button>
          {/each}
        </div>
      {:else if entries.length > 0}
        <div class="tree-list">
          {#each entries as entry}
            {@const Icon = treeIcon(entry)}
            <button class="tree-row {selectedMount?.summary.mount.id === entry.path && entry.kind === 'mount' ? 'active' : ''}" onclick={() => onOpenEntry(entry)}>
              <span class="tree-branch {entry.is_directory ? 'branch-folder' : ''}">
                <Icon size={15} strokeWidth={2} />
              </span>
              <span class="tree-name {isRootEntry(entry) ? 'root-entry' : ''}">{entryLabel(entry)}</span>
              <span class="tree-tags">
                {#if entry.conflict_count}
                  <span class="badge danger">{entry.conflict_count}</span>
                {/if}
                {#if entry.pending_delete_count}
                  <span class="badge warning">{entry.pending_delete_count}</span>
                {/if}
                {#if entry.dirty_count}
                  <span class="badge">{entry.dirty_count}</span>
                {/if}
              </span>
            </button>
          {/each}
        </div>
      {:else}
        <div class="empty-state">这里还没有内容</div>
      {/if}
    </div>

    {#if selectedMount}
      <div class="mount-panel">
        <div class="mount-panel-head">
          <div class="mount-identity">
            <strong>{selectedMount.summary.mount.display_name}</strong>
            <div class="mount-stat-row">
              <span>{selectedMount.summary.dirty_count} 变更</span>
              <span>{selectedMount.summary.conflict_count} 冲突</span>
              <span>{selectedMount.summary.pending_delete_count} 待删</span>
            </div>
          </div>
          <div class="mount-toolbar">
            <button
              class="icon-button"
              onclick={() =>
                onCreateCheckpoint(
                  selectedMount.summary.mount.id,
                  "Manual checkpoint",
                  "Created from workspace rail"
                )}
              aria-label="创建 checkpoint"
            >
              <GitBranch size={16} strokeWidth={2} />
            </button>
            <button class="action-button primary" onclick={() => onKeepMount(selectedMount.summary.mount.id)}>
              <Check size={14} strokeWidth={2} />
              保留全部
            </button>
            <button class="action-button secondary" onclick={() => onRevertMount(selectedMount.summary.mount.id)}>
              <Undo2 size={14} strokeWidth={2} />
              撤销全部
            </button>
          </div>
        </div>

        <div class="checkpoint-strip">
          {#if selectedMount.checkpoints.length === 0}
            <span class="empty-inline">还没有 checkpoint</span>
          {:else}
            {#each selectedMount.checkpoints.slice(0, 6) as checkpoint}
              <button
                class="checkpoint-pill {checkpoint.is_auto ? 'auto' : 'manual'}"
                onclick={() => onRevertMount(selectedMount.summary.mount.id, undefined, checkpoint.id)}
              >
                <span>{checkpoint.label ?? (checkpoint.is_auto ? "Auto" : "Checkpoint")}</span>
                <small>{checkpoint.changed_files.length} files</small>
              </button>
            {/each}
          {/if}
        </div>

        <div class="diff-list">
          {#if sortedDiffs.length === 0}
            <p class="empty-state">当前没有待处理变更。</p>
          {:else}
            {#each sortedDiffs as diff}
              <article class="diff-card {diff.status === 'conflicted' ? 'conflicted' : ''}">
                <div class="diff-card-head">
                  <div>
                    <strong>{diff.path}</strong>
                    <p>{statusLabel(diff.status)}</p>
                  </div>
                  {#if diff.status === "conflicted"}
                    <span class="conflict-chip">
                      <AlertTriangle size={13} strokeWidth={2} />
                      需要处理
                    </span>
                  {/if}
                </div>

                {#if diff.conflict_reason}
                  <div class="notice danger">{diff.conflict_reason}</div>
                {/if}

                {#if diff.status === "conflicted"}
                  <div class="conflict-actions">
                    <button
                      class="action-button"
                      onclick={() => onResolveConflict(selectedMount.summary.mount.id, diff.path, "keep_disk")}
                    >
                      保留磁盘版本
                    </button>
                    <button
                      class="action-button"
                      onclick={() => onResolveConflict(selectedMount.summary.mount.id, diff.path, "keep_workspace")}
                    >
                      保留工作区版本
                    </button>
                  </div>

                  {#if diff.is_binary}
                    <div class="binary-box">
                      二进制冲突无法自动 merge。可以保留磁盘版本、保留工作区版本，或者写入一份副本。
                    </div>
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
                        <button
                          class="action-button secondary"
                          onclick={() => (mergeDrafts[diff.path] = createMergeDraft(diff))}
                        >
                          重置草稿
                        </button>
                      </div>
                      <textarea bind:value={mergeDrafts[diff.path]} rows="12"></textarea>
                      <button
                        class="action-button primary"
                        onclick={() =>
                          onResolveConflict(
                            selectedMount.summary.mount.id,
                            diff.path,
                            "manual_merge",
                            undefined,
                            mergeDrafts[diff.path]
                          )}
                      >
                        <Save size={14} strokeWidth={2} />
                        保存合并结果
                      </button>
                    </div>
                  {/if}

                  <div class="copy-row">
                    <input type="text" bind:value={copyDrafts[diff.path]} placeholder="冲突副本路径..." />
                    <button
                      class="action-button secondary"
                      onclick={() =>
                        onResolveConflict(
                          selectedMount.summary.mount.id,
                          diff.path,
                          "write_copy",
                          copyDrafts[diff.path]
                        )}
                    >
                      写为副本
                    </button>
                  </div>
                {:else}
                  {#if diff.diff_text}
                    <pre>{diff.diff_text}</pre>
                  {:else if diff.working_content}
                    <pre>{diff.working_content}</pre>
                  {:else}
                    <div class="binary-box">二进制或删除变更</div>
                  {/if}

                  <div class="diff-actions">
                    <button
                      class="action-button primary"
                      onclick={() => onKeepMount(selectedMount.summary.mount.id, diff.path)}
                    >
                      保留文件
                    </button>
                    <button
                      class="action-button secondary"
                      onclick={() => onRevertMount(selectedMount.summary.mount.id, diff.path)}
                    >
                      撤销文件
                    </button>
                  </div>
                {/if}
              </article>
            {/each}
          {/if}
        </div>
      </div>
    {/if}
  </div>

</aside>

<style>
  .right-sidebar {
    position: relative;
    width: 380px;
    background:
      radial-gradient(circle at top right, rgba(214, 184, 108, 0.24), transparent 30%),
      linear-gradient(180deg, #fbf7ee 0%, #f5efe1 100%);
    border-left: 1px solid rgba(74, 56, 33, 0.12);
    display: flex;
    flex-direction: column;
    padding: 18px;
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
    gap: 14px;
    min-height: 0;
    min-width: 320px;
  }

  .workspace-header,
  .mount-panel-head,
  .header-actions,
  .title-cluster {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .title-cluster {
    min-width: 0;
    flex: 1;
    flex-direction: column;
    align-items: flex-start;
  }

  .workspace-title {
    font-size: 14px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: #513d1f;
  }

  .path-strip {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    flex-wrap: wrap;
    color: #7a6442;
  }

  .path-segment {
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }

  .icon-button,
  .action-button,
  .tree-row,
  .checkpoint-pill {
    border: 0;
    cursor: pointer;
    transition:
      transform 0.14s ease,
      opacity 0.14s ease,
      background 0.14s ease,
      border-color 0.14s ease;
  }

  .icon-button:hover,
  .action-button:hover,
  .tree-row:hover,
  .checkpoint-pill:hover {
    transform: translateY(-1px);
  }

  .icon-button {
    width: 34px;
    height: 34px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 12px;
    background: rgba(122, 94, 52, 0.08);
    color: #5c4828;
  }

  .action-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    min-height: 34px;
    padding: 0 12px;
    border-radius: 11px;
    font-size: 12px;
    font-weight: 600;
  }

  .action-button.primary {
    background: #5c4828;
    color: #fff9ee;
  }

  .action-button.secondary,
  .action-button {
    background: rgba(122, 94, 52, 0.11);
    color: #5c4828;
  }

  .search-box,
  .tree-shell,
  .mount-panel {
    background: rgba(255, 252, 244, 0.88);
    border: 1px solid rgba(92, 72, 40, 0.1);
    border-radius: 18px;
  }

  .search-box {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 14px;
  }

  .tree-shell,
  .mount-panel {
    padding: 12px;
  }

  .tree-shell,
  .mount-panel,
  .diff-list {
    min-height: 0;
  }

  .tree-shell {
    display: flex;
    flex-direction: column;
    gap: 6px;
    flex: 0 1 36%;
    min-height: 180px;
  }

  .tree-list,
  .diff-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    overflow: auto;
  }

  .tree-row {
    width: 100%;
    display: grid;
    grid-template-columns: 22px minmax(0, 1fr) auto;
    align-items: center;
    gap: 10px;
    padding: 9px 10px;
    border-radius: 12px;
    background: rgba(122, 94, 52, 0.04);
    color: #3f301d;
    text-align: left;
  }

  .tree-row.active {
    background: rgba(92, 72, 40, 0.12);
    box-shadow: inset 0 0 0 1px rgba(92, 72, 40, 0.12);
  }

  .tree-row.search-row {
    grid-template-columns: 22px minmax(0, 1fr);
  }

  .back-row {
    background: transparent;
    border: 1px dashed rgba(122, 94, 52, 0.18);
  }

  .tree-branch {
    width: 22px;
    height: 22px;
    border-radius: 8px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: rgba(122, 94, 52, 0.08);
    color: #6b5433;
  }

  .tree-branch.branch-folder {
    background: rgba(201, 150, 57, 0.14);
    color: #8f6821;
  }

  .tree-name,
  .tree-label {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .tree-name {
    font-size: 13px;
    font-weight: 600;
    color: #3f301d;
  }

  .root-entry {
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: #6c5635;
  }

  .tree-label strong {
    font-size: 13px;
    color: #3f301d;
  }

  .tree-label span {
    font-size: 12px;
    color: #836949;
  }

  .tree-tags {
    display: inline-flex;
    gap: 6px;
    align-items: center;
  }

  .badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 24px;
    height: 20px;
    padding: 0 6px;
    border-radius: 999px;
    background: rgba(92, 72, 40, 0.12);
    color: #5c4828;
    font-size: 11px;
    font-weight: 700;
  }

  .badge.warning {
    background: rgba(207, 140, 60, 0.18);
    color: #955f1c;
  }

  .badge.danger {
    background: rgba(194, 63, 63, 0.16);
    color: #9a2f2f;
  }

  .mount-panel {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 12px;
    min-height: 0;
  }

  .mount-panel-head {
    align-items: flex-start;
  }

  .mount-identity {
    min-width: 0;
  }

  .mount-identity strong {
    color: #3f301d;
  }

  .mount-stat-row,
  .empty-inline,
  .diff-card-head p {
    margin-top: 4px;
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    font-size: 12px;
    color: #836949;
  }

  .mount-toolbar {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 8px;
  }

  .checkpoint-strip {
    display: flex;
    gap: 8px;
    overflow: auto;
    padding-bottom: 2px;
  }

  .checkpoint-pill {
    display: inline-flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    min-width: 110px;
    padding: 10px 12px;
    border-radius: 14px;
    background: rgba(122, 94, 52, 0.08);
    color: #5c4828;
  }

  .checkpoint-pill.auto {
    background: rgba(121, 123, 185, 0.14);
    color: #4a4f88;
  }

  .checkpoint-pill.manual {
    background: rgba(203, 162, 71, 0.16);
    color: #845f16;
  }

  .checkpoint-pill small {
    font-size: 11px;
    opacity: 0.8;
  }

  .diff-card {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 14px;
    border-radius: 16px;
    background: rgba(122, 94, 52, 0.06);
  }

  .diff-card.conflicted {
    box-shadow: inset 0 0 0 1px rgba(177, 72, 72, 0.24);
    background: rgba(194, 63, 63, 0.05);
  }

  .diff-card-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }

  .diff-card-head strong {
    font-size: 13px;
    color: #3f301d;
  }

  .conflict-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border-radius: 999px;
    padding: 6px 10px;
    background: rgba(194, 63, 63, 0.13);
    color: #9a2f2f;
    font-size: 11px;
    font-weight: 700;
  }

  .notice {
    border-radius: 13px;
    padding: 10px 12px;
    font-size: 12px;
  }

  .notice.danger {
    background: rgba(194, 63, 63, 0.1);
    color: #8f2f2f;
  }

  .conflict-actions,
  .diff-actions,
  .copy-row,
  .merge-head {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }

  .conflict-columns {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 10px;
  }

  .conflict-columns section,
  .binary-box {
    border-radius: 13px;
    background: rgba(255, 255, 255, 0.7);
    border: 1px solid rgba(92, 72, 40, 0.1);
    padding: 10px;
  }

  .conflict-columns section span {
    display: inline-block;
    margin-bottom: 8px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: #836949;
  }

  pre,
  textarea,
  input {
    box-sizing: border-box;
    width: 100%;
    border: 1px solid rgba(92, 72, 40, 0.14);
    border-radius: 12px;
    background: rgba(255, 255, 255, 0.86);
    padding: 10px 12px;
    font: inherit;
    color: #3b2b18;
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
    min-height: 220px;
    resize: vertical;
  }

  .manual-merge {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .empty-state {
    padding: 18px 12px;
    color: #836949;
    font-size: 12px;
  }

  @media (max-width: 1024px) {
    .right-sidebar {
      width: min(42vw, 360px);
    }

    .conflict-columns {
      grid-template-columns: 1fr;
    }
  }
</style>
