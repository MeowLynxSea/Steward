<script lang="ts">
  import {
    AlertTriangle,
    Check,
    ChevronRight,
    FileText,
    Folder,
    FolderPlus,
    GitBranch,
    Save,
    Search,
    Undo2,
    X
  } from "lucide-svelte";
  import type {
    AllowlistedFileDiff,
    WorkspaceChangeGroup,
    AllowlistedFileStatus,
    WorkspaceDocumentView,
    WorkspaceEntry,
    WorkspaceAllowlistDetail,
    WorkspaceAllowlistFileView,
    WorkspaceSearchResult
  } from "../lib/types";

  type ConflictResolution = "keep_disk" | "keep_workspace" | "write_copy" | "manual_merge";
  type WorkspaceTab = "files" | "changes";

  interface Props {
    currentPath: string;
    entries: WorkspaceEntry[];
    searchResults: WorkspaceSearchResult[];
    searchQuery: string;
    selectedAllowlist: WorkspaceAllowlistDetail | null;
    selectedFile: WorkspaceAllowlistFileView | null;
    selectedDocument: WorkspaceDocumentView | null;
    changeGroups: WorkspaceChangeGroup[];
    loading: boolean;
    fileLoading: boolean;
    searchLoading: boolean;
    busyAction: string | null;
    collapsed?: boolean;
    onSearch: (query: string) => void;
    onClearSearch: () => void;
    onClearPreview: () => void;
    onNavigate: (path: string) => void;
    onOpenEntry: (entry: WorkspaceEntry) => void;
    onOpenChangesTab?: () => void;
    onRequestAllowlist: () => void;
    onKeepAllowlist: (allowlistId: string, scopePath?: string, checkpointId?: string) => void;
    onRevertAllowlist: (allowlistId: string, scopePath?: string, checkpointId?: string) => void;
    onCreateCheckpoint: (allowlistId: string, label?: string, summary?: string) => void;
    onResolveConflict: (
      allowlistId: string,
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
    selectedAllowlist,
    selectedFile,
    selectedDocument,
    changeGroups,
    loading,
    fileLoading,
    searchLoading,
    busyAction,
    collapsed = false,
    onSearch,
    onClearSearch,
    onClearPreview,
    onNavigate,
    onOpenEntry,
    onOpenChangesTab,
    onRequestAllowlist,
    onKeepAllowlist,
    onRevertAllowlist,
    onCreateCheckpoint,
    onResolveConflict,
    onUseResult
  }: Props = $props();

  let localQuery = $state("");
  let activeTab = $state<WorkspaceTab>("files");
  let mergeDrafts = $state<Record<string, string>>({});
  let copyDrafts = $state<Record<string, string>>({});

  const sortedChangeGroups = $derived(
    [...changeGroups]
      .map((group) => ({
        ...group,
        entries: [...group.entries].sort((left, right) => {
          const weight = (status: AllowlistedFileStatus) => {
            if (status === "conflicted") return 0;
            if (status === "deleted") return 1;
            if (status === "binary_modified") return 2;
            return 3;
          };
          return weight(left.status) - weight(right.status) || left.path.localeCompare(right.path);
        })
      }))
      .sort((left, right) =>
        left.allowlist.summary.allowlist.display_name.localeCompare(right.allowlist.summary.allowlist.display_name)
      )
  );
  const sortedEntries = $derived(
    [...entries].sort((left, right) => {
      if (left.is_directory !== right.is_directory) {
        return left.is_directory ? -1 : 1;
      }
      return entryLabel(left).localeCompare(entryLabel(right), undefined, { numeric: true });
    })
  );

  const breadcrumbs = $derived(buildBreadcrumbs(currentPath, selectedAllowlist?.summary.allowlist.display_name ?? null));
  const previewOpen = $derived(fileLoading || Boolean(selectedDocument) || Boolean(selectedFile));

  $effect(() => {
    localQuery = searchQuery;
    for (const group of changeGroups) {
      for (const diff of group.entries) {
        const key = `${group.allowlist.summary.allowlist.id}:${diff.path}`;
        if (diff.status === "conflicted" && !mergeDrafts[key]) {
          mergeDrafts[key] = createMergeDraft(diff);
        }
        if (diff.status === "conflicted" && !copyDrafts[key]) {
          copyDrafts[key] = `${diff.path}.workspace-copy`;
        }
      }
    }
  });

  function handleSearch() {
    onSearch(localQuery);
  }

  function closePreview() {
    onClearPreview();
  }

  function jumpToChanges() {
    activeTab = "changes";
    onOpenChangesTab?.();
    closePreview();
  }

  function activateChangesTab() {
    activeTab = "changes";
    onOpenChangesTab?.();
  }

  function allowlistIdFromUri(uri: string) {
    if (!uri.startsWith("workspace://")) {
      return null;
    }

    const remainder = uri.slice("workspace://".length);
    if (!remainder) {
      return null;
    }

    const [allowlistId] = remainder.split("/", 1);
    return allowlistId || null;
  }

  function buildBreadcrumbs(path: string, allowlistName: string | null) {
    const root = [{ label: "Workspace", path: "workspace://" }];
    if (path === "workspace://") {
      return root;
    }

    if (!path.startsWith("workspace://")) {
      const trail = [...root];
      const segments = path.split("/").filter(Boolean);
      let cursor = "";
      for (const segment of segments) {
        cursor = cursor ? `${cursor}/${segment}` : segment;
        trail.push({ label: segment, path: cursor });
      }
      return trail;
    }

    const remainder = path.slice("workspace://".length);
    const segments = remainder.split("/").filter(Boolean);
    const [allowlistId, ...subpaths] = segments;
    const trail = [...root, { label: allowlistName ?? allowlistId, path: `workspace://${allowlistId}` }];
    let cursor = `workspace://${allowlistId}`;
    for (const segment of subpaths) {
      cursor = `${cursor}/${segment}`;
      trail.push({ label: segment, path: cursor });
    }
    return trail;
  }

  function createMergeDraft(diff: AllowlistedFileDiff) {
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

  function statusLabel(status: AllowlistedFileStatus) {
    switch (status) {
      case "clean": return "已同步";
      case "conflicted": return "冲突";
      case "deleted": return "待删除";
      case "binary_modified": return "二进制修改";
      case "added": return "新增";
      case "modified": return "修改";
    }
  }

  function entryLabel(entry: WorkspaceEntry) {
    return entry.name ?? entry.path;
  }

  function treeIcon(entry: WorkspaceEntry) {
    if (entry.kind === "allowlist") return Folder;
    return entry.is_directory ? Folder : FileText;
  }

  function isEntryActive(entry: WorkspaceEntry) {
    const uri = entry.uri ?? entry.path;
    if (entry.kind === "allowlist") {
      return currentPath === uri || currentPath.startsWith(`${uri}/`);
    }
    if (entry.is_directory) {
      return currentPath === uri || currentPath.startsWith(`${uri}/`);
    }
    if (uri.startsWith("workspace://")) {
      const allowlistId = allowlistIdFromUri(uri);
      return selectedFile?.allowlist_id === allowlistId && selectedFile.path === entry.path;
    }
    return selectedDocument?.path === uri;
  }

  function previewHeadline(file: WorkspaceAllowlistFileView) {
    switch (file.status) {
      case "clean":
        return "当前磁盘内容";
      case "modified":
        return "文件有未提交变更";
      case "added":
        return "文件是工作区新增内容";
      case "deleted":
        return "文件标记为删除";
      case "conflicted":
        return "文件存在冲突";
      case "binary_modified":
        return "二进制文件已修改";
    }
  }

  function previewBody(file: WorkspaceAllowlistFileView) {
    switch (file.status) {
      case "clean":
        return file.is_binary
          ? "这是一个二进制文件，当前预览不可用。"
          : "只读预览来自当前授权目录文件。";
      case "modified":
        return "请切换到 Changes 标签页查看 diff，并决定保留或撤销。";
      case "added":
        return "这个文件还没有提交到磁盘基线，请在 Changes 标签页完成处理。";
      case "deleted":
        return "这个文件已被标记为删除，正文预览已停用。";
      case "conflicted":
        return "这个文件同时发生了磁盘变更和工作区变更，请到 Changes 标签页解决冲突。";
      case "binary_modified":
        return "这是一个二进制改动，正文预览不可用，请在 Changes 标签页决定保留或撤销。";
    }
  }

  function diffKey(allowlistId: string, path: string) {
    return `${allowlistId}:${path}`;
  }
</script>

<aside class="right-sidebar {collapsed ? 'collapsed' : ''}">
  <div class="right-sidebar-inner">
    <div class="tab-bar">
      <div class="tab-actions">
        <button class="tab {activeTab === 'files' ? 'active' : ''}" onclick={() => activeTab = 'files'}>
          文件
        </button>
        <button class="tab {activeTab === 'changes' ? 'active' : ''}" onclick={activateChangesTab}>
          变更
        </button>
        {#if busyAction}
          <span class="workspace-status">{busyAction}</span>
        {/if}
      </div>
      <button class="icon-btn" onclick={onRequestAllowlist} aria-label="授权目录">
        <FolderPlus size={15} strokeWidth={2} />
      </button>
    </div>

    {#if activeTab === "files"}
      <div class="search-box">
        <Search size={14} strokeWidth={2} />
        <input
          type="text"
          placeholder="搜索工作区内容..."
          bind:value={localQuery}
          onkeydown={(event) => event.key === "Enter" && handleSearch()}
        />
        {#if localQuery.trim()}
          <button class="clear-search" onclick={onClearSearch} aria-label="清空搜索">
            <X size={12} strokeWidth={2} />
          </button>
        {/if}
      </div>

      {#if breadcrumbs.length > 0}
        <div class="breadcrumb-row">
          {#each breadcrumbs as crumb, index}
            <button class="breadcrumb" onclick={() => onNavigate(crumb.path)}>
              {crumb.label}
            </button>
            {#if index < breadcrumbs.length - 1}
              <ChevronRight size={12} strokeWidth={2} />
            {/if}
          {/each}
        </div>
      {/if}

      <div class="files-panel">
        <div class="file-tree">
          {#if searchLoading}
            <div class="empty-hint">正在搜索...</div>
          {:else if searchResults.length > 0}
            {#each searchResults.slice(0, 10) as result}
              <button class="search-result" onclick={() => onUseResult(result)}>
                <div class="search-result-head">
                  <Search size={13} strokeWidth={2} />
                  <span>{result.document_path}</span>
                </div>
                <p>{result.content}</p>
              </button>
            {/each}
          {:else if loading}
            <div class="empty-hint">正在加载目录...</div>
          {:else if entries.length === 0}
            <div class="empty-hint">这里还没有内容。</div>
          {:else}
            {#each sortedEntries as entry}
              {@const Icon = treeIcon(entry)}
              <button
                class="tree-item {isEntryActive(entry) ? 'active' : ''}"
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
                    {#if entry.pending_delete_count}
                      <span class="badge muted">{entry.pending_delete_count}</span>
                    {/if}
                  </span>
                {/if}
              </button>
            {/each}
          {/if}
        </div>
      </div>
    {:else}
      <div class="changes-panel">
        {#if sortedChangeGroups.length === 0}
          <div class="empty-hint">当前没有待处理变更</div>
        {:else}
          <div class="diff-list">
            {#each sortedChangeGroups as group}
              <section class="allowlist-group">
                <div class="allowlist-info">
                  <div>
                    <strong>{group.allowlist.summary.allowlist.display_name}</strong>
                    <div class="allowlist-stats">
                      <span>{group.allowlist.summary.dirty_count} 变更</span>
                      <span>{group.allowlist.summary.conflict_count} 冲突</span>
                      <span>{group.allowlist.summary.pending_delete_count} 删除</span>
                    </div>
                  </div>
                  <div class="allowlist-actions">
                    <button
                      class="action-btn"
                      onclick={() => onCreateCheckpoint(
                        group.allowlist.summary.allowlist.id,
                        "Manual checkpoint",
                        "Created from workspace rail"
                      )}
                      aria-label="创建 checkpoint"
                    >
                      <GitBranch size={13} strokeWidth={2} />
                    </button>
                    <button class="action-btn primary" onclick={() => onKeepAllowlist(group.allowlist.summary.allowlist.id)}>
                      <Check size={12} strokeWidth={2} />
                      保留全部
                    </button>
                    <button class="action-btn" onclick={() => onRevertAllowlist(group.allowlist.summary.allowlist.id)}>
                      <Undo2 size={12} strokeWidth={2} />
                      撤销全部
                    </button>
                  </div>
                </div>

                {#if group.allowlist.checkpoints.length > 0}
                  <div class="checkpoint-row">
                    {#each group.allowlist.checkpoints.slice(0, 4) as checkpoint}
                      <button
                        class="checkpoint-pill {checkpoint.is_auto ? 'auto' : 'manual'}"
                        onclick={() => onRevertAllowlist(group.allowlist.summary.allowlist.id, undefined, checkpoint.id)}
                      >
                        <span>{checkpoint.label ?? (checkpoint.is_auto ? "Auto" : "Checkpoint")}</span>
                        <small>{checkpoint.changed_files.length} files</small>
                      </button>
                    {/each}
                  </div>
                {/if}

                {#each group.entries as diff}
                  {@const key = diffKey(group.allowlist.summary.allowlist.id, diff.path)}
                  <article class="diff-card {diff.status === 'conflicted' ? 'conflicted' : ''}">
                    <div class="diff-card-head">
                      <div class="diff-card-meta">
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
                        <button class="action-btn" onclick={() => onResolveConflict(group.allowlist.summary.allowlist.id, diff.path, "keep_disk")}>
                          保留磁盘版本
                        </button>
                        <button class="action-btn" onclick={() => onResolveConflict(group.allowlist.summary.allowlist.id, diff.path, "keep_workspace")}>
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
                            <button class="action-btn" onclick={() => (mergeDrafts[key] = createMergeDraft(diff))}>
                              重置草稿
                            </button>
                          </div>
                          <textarea bind:value={mergeDrafts[key]} rows="10"></textarea>
                          <button
                            class="action-btn primary"
                            onclick={() => onResolveConflict(
                              group.allowlist.summary.allowlist.id,
                              diff.path,
                              "manual_merge",
                              undefined,
                              mergeDrafts[key]
                            )}
                          >
                            <Save size={13} strokeWidth={2} />
                            保存合并结果
                          </button>
                        </div>
                      {/if}

                      <div class="copy-row">
                        <input type="text" bind:value={copyDrafts[key]} placeholder="冲突副本路径..." />
                        <button class="action-btn" onclick={() => onResolveConflict(
                          group.allowlist.summary.allowlist.id,
                          diff.path,
                          "write_copy",
                          copyDrafts[key]
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
                        <button class="action-btn primary" onclick={() => onKeepAllowlist(group.allowlist.summary.allowlist.id, diff.path)}>
                          保留文件
                        </button>
                        <button class="action-btn" onclick={() => onRevertAllowlist(group.allowlist.summary.allowlist.id, diff.path)}>
                          撤销文件
                        </button>
                      </div>
                    {/if}
                  </article>
                {/each}
              </section>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </div>
</aside>

{#if previewOpen}
  <div
    class="preview-modal-backdrop"
    role="presentation"
    onclick={closePreview}
    onkeydown={(event) => event.key === "Escape" && closePreview()}
  >
    <div
      class="preview-modal"
      role="dialog"
      aria-modal="true"
      aria-label="文件预览"
      tabindex="-1"
    >
      <div class="preview-modal-inner" role="presentation" onclick={(event) => event.stopPropagation()}>
        <div class="preview-modal-head">
          <div class="preview-modal-title">
            <strong>{selectedDocument?.path ?? selectedFile?.path ?? "文件预览"}</strong>
            <span>
              {#if selectedDocument}
                工作区文件
              {:else if selectedFile}
                {statusLabel(selectedFile.status)}
              {:else}
                正在加载
              {/if}
            </span>
          </div>
          <button class="icon-btn" onclick={closePreview} aria-label="关闭预览">
            <X size={16} strokeWidth={2} />
          </button>
        </div>

        <div class="preview-modal-body">
          {#if fileLoading}
            <div class="preview-empty">正在加载文件预览...</div>
          {:else if selectedDocument}
            <p class="preview-meta">只读预览当前工作区文档内容。</p>
            <pre>{selectedDocument.content || "(empty file)"}</pre>
          {:else if selectedFile}
            <p class="preview-meta">{previewHeadline(selectedFile)}</p>

            {#if selectedFile.status === "clean" && !selectedFile.is_binary}
              <pre>{selectedFile.content ?? "(empty file)"}</pre>
            {:else}
              <div class="preview-summary">
                <p>{previewBody(selectedFile)}</p>
                {#if selectedFile.status !== "clean"}
                  <button class="jump-button" onclick={jumpToChanges}>
                    跳到 Changes
                  </button>
                {/if}
              </div>
            {/if}
          {/if}
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .right-sidebar {
    width: 340px;
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
    min-width: 300px;
    padding: 12px;
    gap: 8px;
  }

  .allowlist-actions,
  .merge-head,
  .diff-item-actions,
  .conflict-actions,
  .copy-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .workspace-status,
  .preview-meta,
  .allowlist-stats,
  .diff-status,
  .empty-hint,
  .preview-empty {
    color: var(--text-muted);
    font-size: 12px;
  }

  .tree-item-badges,
  .tab-actions {
    display: flex;
    gap: 6px;
  }

  .icon-btn,
  .tab,
  .tree-item,
  .search-result,
  .action-btn,
  .checkpoint-pill,
  .breadcrumb,
  .clear-search,
  .jump-button {
    border: none;
    cursor: pointer;
    transition: background 0.15s ease, color 0.15s ease, transform 0.15s ease;
  }

  .icon-btn,
  .clear-search {
    width: 30px;
    height: 30px;
    border-radius: 8px;
    background: transparent;
    color: var(--text-tertiary);
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .icon-btn:hover,
  .clear-search:hover,
  .tree-item:hover,
  .search-result:hover,
  .action-btn:hover,
  .checkpoint-pill:hover,
  .jump-button:hover {
    background: var(--bg-hover);
  }

  .tab-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0;
    border-bottom: 1px solid var(--border-default);
  }

  .tab {
    padding: 6px 12px;
    font-size: 12px;
    color: var(--text-tertiary);
    background: transparent;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }

  .tab.active {
    color: var(--text-primary);
    border-bottom-color: var(--accent-primary);
  }

  .search-box {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-radius: 9px;
    background: var(--bg-input);
    color: var(--text-tertiary);
  }

  .search-box input,
  .copy-row input,
  .manual-merge textarea {
    flex: 1;
    background: transparent;
    border: none;
    color: var(--text-primary);
    font: inherit;
    outline: none;
  }

  .search-box input {
    font-size: 12px;
    line-height: 1.3;
  }

  .breadcrumb-row {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
    min-height: 22px;
  }

  .breadcrumb {
    background: transparent;
    color: var(--text-secondary);
    font-size: 12px;
    padding: 0;
  }

  .files-panel,
  .changes-panel,
  .diff-list {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    gap: 8px;
  }

  .file-tree,
  .diff-list {
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .file-tree {
    flex: 1;
  }

  .tree-item,
  .search-result {
    width: 100%;
    text-align: left;
    border-radius: 10px;
    padding: 8px 10px;
    background: transparent;
    color: var(--text-primary);
  }

  .tree-item {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .tree-item.active {
    background: var(--bg-active);
  }

  .tree-item-icon {
    color: var(--text-tertiary);
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
  }

  .tree-item-icon.folder {
    color: var(--accent-gold);
  }

  .tree-item-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: 12px;
    line-height: 1.3;
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

  .badge.muted {
    background: var(--bg-hover);
  }

  .search-result {
    border: 1px solid var(--border-default);
    background: var(--bg-surface);
  }

  .search-result-head {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    font-weight: 600;
  }

  .search-result p {
    margin: 8px 0 0;
    font-size: 12px;
    color: var(--text-secondary);
    line-height: 1.5;
  }

  .allowlist-info,
  .allowlist-group,
  .diff-card,
  .notice,
  .binary-notice,
  .manual-merge,
  .copy-row,
  .preview-summary {
    border: 1px solid var(--border-default);
    border-radius: 12px;
    background: var(--bg-surface);
  }

  .search-result-head,
  .diff-card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }

  .preview-modal-body pre,
  .diff-card pre,
  .conflict-columns pre {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    overflow: auto;
    font-size: 12px;
    line-height: 1.55;
    color: var(--text-primary);
  }

  .preview-summary,
  .preview-empty {
    padding: 12px;
  }

  .jump-button,
  .action-btn,
  .checkpoint-pill {
    border-radius: 9px;
    padding: 7px 9px;
    background: var(--bg-hover);
    color: var(--text-primary);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    font-size: 11px;
  }

  .action-btn.primary,
  .jump-button {
    background: var(--accent-primary);
    color: var(--text-on-dark);
  }

  .allowlist-info,
  .diff-card,
  .manual-merge,
  .copy-row {
    padding: 10px;
  }

  .allowlist-group {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px;
  }

  .allowlist-stats {
    display: flex;
    gap: 8px;
    margin-top: 2px;
  }

  .checkpoint-row {
    display: flex;
    gap: 6px;
    overflow-x: auto;
    padding-bottom: 2px;
  }

  .checkpoint-pill {
    flex-direction: column;
    align-items: flex-start;
    min-width: 112px;
  }

  .checkpoint-pill small {
    color: var(--text-muted);
    font-size: 10px;
  }

  .checkpoint-pill.auto {
    border: 1px dashed var(--border-input);
  }

  .diff-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .diff-card-meta {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 2px;
  }

  .diff-card-meta strong,
  .allowlist-info strong {
    font-size: 12px;
    line-height: 1.3;
  }

  .diff-card.conflicted {
    border-color: var(--accent-danger);
  }

  .conflict-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border-radius: 999px;
    background: rgba(220, 38, 38, 0.12);
    color: var(--accent-danger);
    font-size: 11px;
    font-weight: 700;
  }

  .notice,
  .binary-notice {
    padding: 8px 10px;
    font-size: 11px;
    color: var(--text-secondary);
  }

  .notice.danger {
    border-color: rgba(220, 38, 38, 0.25);
    background: rgba(220, 38, 38, 0.08);
  }

  .conflict-columns {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 6px;
  }

  .conflict-columns section {
    border-radius: 10px;
    background: var(--bg-hover);
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }

  .conflict-columns span,
  .merge-head strong {
    font-size: 12px;
    font-weight: 700;
    color: var(--text-secondary);
  }

  .manual-merge {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .manual-merge textarea {
    min-height: 180px;
    resize: vertical;
  }

  .copy-row {
    gap: 8px;
  }

  .preview-modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 70;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 28px;
    background: rgba(9, 12, 20, 0.44);
    backdrop-filter: blur(10px);
  }

  .preview-modal {
    width: min(1100px, calc(100vw - 56px));
    height: min(82vh, 900px);
    border-radius: 24px;
    border: 1px solid var(--border-default);
    background: var(--bg-surface);
    box-shadow: var(--shadow-dropdown);
    overflow: hidden;
  }

  .preview-modal-inner {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .preview-modal-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 18px 20px;
    border-bottom: 1px solid var(--border-default);
  }

  .preview-modal-title {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 4px;
  }

  .preview-modal-title strong {
    color: var(--text-primary);
    font-size: 15px;
  }

  .preview-modal-title span {
    color: var(--text-muted);
    font-size: 12px;
  }

  .preview-modal-body {
    display: flex;
    flex: 1;
    min-height: 0;
    flex-direction: column;
    gap: 12px;
    padding: 18px 20px 20px;
    overflow: auto;
  }

  @media (max-width: 1100px) {
    .right-sidebar {
      width: 320px;
    }

    .conflict-columns {
      grid-template-columns: 1fr;
    }

    .preview-modal {
      width: calc(100vw - 24px);
      height: calc(100vh - 24px);
      border-radius: 18px;
    }

    .preview-modal-backdrop {
      padding: 12px;
    }
  }
</style>
