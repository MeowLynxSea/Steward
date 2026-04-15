<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import {
    AlertTriangle,
    ArrowDown,
    ArrowUp,
    Check,
    ChevronRight,
    Clock,
    FileText,
    Folder,
    FolderPlus,
    GitBranch,
    RotateCcw,
    Save,
    Search,
    Trash2,
    Undo2,
    X
  } from "lucide-svelte";
  import hljs from "highlight.js/lib/core";
  import { showToast } from "../lib/stores/toast.svelte";
  import type {
    AllowlistedFileDiff,
    WorkspaceAllowlistCheckpoint,
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
    onRestoreCheckpoint: (allowlistId: string, checkpointId: string) => void;
    onDeleteCheckpoint: (allowlistId: string, checkpointId: string) => void;
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
    onRestoreCheckpoint,
    onDeleteCheckpoint,
    onResolveConflict,
    onUseResult
  }: Props = $props();

  let localQuery = $state("");
  let activeTab = $state<WorkspaceTab>("files");
  let mergeDrafts = $state<Record<string, string>>({});
  let copyDrafts = $state<Record<string, string>>({});
  let diffModalFile = $state<{ allowlistId: string; diff: AllowlistedFileDiff } | null>(null);

  // Preview modal code viewer state
  let findBarOpen = $state(false);
  let findQuery = $state("");
  let findMatches = $state<{ line: number; col: number }[]>([]);
  let findIndex = $state(0);
  let gotoLineOpen = $state(false);
  let gotoLineValue = $state("");
  let codeViewerRef = $state<HTMLElement | null>(null);
  let findInputRef = $state<HTMLInputElement | null>(null);
  let gotoInputRef = $state<HTMLInputElement | null>(null);

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
    const root = [{ label: "工作区", path: "workspace://" }];
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

  function statusBadge(status: AllowlistedFileStatus): string {
    switch (status) {
      case "added": return "A";
      case "modified": return "M";
      case "deleted": return "D";
      case "conflicted": return "C";
      case "binary_modified": return "B";
      default: return "U";
    }
  }

  function statusBadgeClass(status: AllowlistedFileStatus): string {
    switch (status) {
      case "added": return "badge-added";
      case "modified": return "badge-modified";
      case "deleted": return "badge-deleted";
      case "conflicted": return "badge-conflict";
      default: return "badge-other";
    }
  }

  function fileName(path: string): string {
    return path.split("/").pop() ?? path;
  }

  function dirName(path: string): string {
    const parts = path.split("/");
    return parts.length > 1 ? parts.slice(0, -1).join("/") : "";
  }

  function diffStats(diffText: string | null): { added: number; removed: number } {
    if (!diffText) return { added: 0, removed: 0 };
    let added = 0;
    let removed = 0;
    for (const line of diffText.split("\n")) {
      if (line.startsWith("+") && !line.startsWith("+++")) added++;
      else if (line.startsWith("-") && !line.startsWith("---")) removed++;
    }
    return { added, removed };
  }

  function parseDiffLines(diffText: string | null): Array<{ type: "add" | "remove" | "context" | "header"; content: string }> {
    if (!diffText) return [];
    return diffText.split("\n").map((line) => {
      if (line.startsWith("@@")) return { type: "header" as const, content: line };
      if (line.startsWith("+") && !line.startsWith("+++")) return { type: "add" as const, content: line.slice(1) };
      if (line.startsWith("-") && !line.startsWith("---")) return { type: "remove" as const, content: line.slice(1) };
      return { type: "context" as const, content: line.startsWith(" ") ? line.slice(1) : line };
    }).filter((l) => !l.content.startsWith("+++") && !l.content.startsWith("---"));
  }

  function openDiffModal(allowlistId: string, diff: AllowlistedFileDiff) {
    diffModalFile = { allowlistId, diff };
  }

  function closeDiffModal() {
    diffModalFile = null;
  }

  function entryLabel(entry: WorkspaceEntry) {
    return entry.name ?? entry.path;
  }

  function entryAllowlistId(entry: WorkspaceEntry) {
    if (entry.kind !== "allowlist") {
      return null;
    }
    const uri = entry.uri ?? entry.path;
    return allowlistIdFromUri(uri);
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
        return "请切换到「变更」标签页查看差异，并决定保留或撤销。";
      case "added":
        return "这个文件尚未纳入工作区基线，请在「变更」标签页决定保留或撤销。";
      case "deleted":
        return "这个文件已被标记为删除，正文预览已停用。";
      case "conflicted":
        return "这个文件同时发生了磁盘变更和工作区变更，请到「变更」标签页解决冲突。";
      case "binary_modified":
        return "这是一个二进制改动，正文预览不可用，请在「变更」标签页决定保留或撤销。";
    }
  }

  function canPreviewTextContent(file: WorkspaceAllowlistFileView) {
    return !file.is_binary && file.status !== "deleted" && file.content !== null;
  }

  // ── Code viewer helpers ──

  const extToLang: Record<string, string> = {
    js: "javascript", jsx: "javascript", mjs: "javascript", cjs: "javascript",
    ts: "typescript", tsx: "typescript", mts: "typescript",
    py: "python", pyw: "python",
    rs: "rust",
    sh: "bash", bash: "bash", zsh: "bash", fish: "bash",
    json: "json", jsonc: "json", json5: "json",
    css: "css", scss: "css", less: "css",
    html: "html", htm: "html", svelte: "html", vue: "html",
    xml: "xml", svg: "xml",
    sql: "sql",
    yaml: "yaml", yml: "yaml", toml: "yaml",
    md: "markdown", mdx: "markdown",
    diff: "diff", patch: "diff",
    go: "go",
    java: "java",
    cpp: "cpp", c: "cpp", cc: "cpp", cxx: "cpp", h: "cpp", hpp: "cpp",
  };

  function detectLanguage(path: string | undefined): string | null {
    if (!path) return null;
    const ext = path.split(".").pop()?.toLowerCase();
    if (!ext) return null;
    return extToLang[ext] ?? null;
  }

  function highlightCode(code: string, lang: string | null): string {
    if (lang && hljs.getLanguage(lang)) {
      return hljs.highlight(code, { language: lang }).value;
    }
    return code
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
  }

  function getPreviewContent(): string | null {
    if (selectedDocument?.content) return selectedDocument.content;
    if (selectedFile && canPreviewTextContent(selectedFile)) return selectedFile.content;
    return null;
  }

  function getPreviewPath(): string | undefined {
    return selectedDocument?.path ?? selectedFile?.path;
  }

  const previewLines = $derived.by(() => {
    const content = getPreviewContent();
    if (!content) return null;
    return content.split("\n");
  });

  const highlightedLines = $derived.by(() => {
    if (!previewLines) return null;
    const lang = detectLanguage(getPreviewPath());
    const full = highlightCode(previewLines.join("\n"), lang);
    return full.split("\n");
  });

  const lineNumberWidth = $derived(
    previewLines ? String(previewLines.length).length : 1
  );

  function openFindBar() {
    findBarOpen = true;
    gotoLineOpen = false;
    requestAnimationFrame(() => findInputRef?.focus());
  }

  function closeFindBar() {
    findBarOpen = false;
    findQuery = "";
    findMatches = [];
    findIndex = 0;
  }

  function openGotoLine() {
    gotoLineOpen = true;
    findBarOpen = false;
    gotoLineValue = "";
    requestAnimationFrame(() => gotoInputRef?.focus());
  }

  function closeGotoLine() {
    gotoLineOpen = false;
    gotoLineValue = "";
  }

  function performFind() {
    if (!findQuery || !previewLines) {
      findMatches = [];
      findIndex = 0;
      return;
    }
    const q = findQuery.toLowerCase();
    const matches: { line: number; col: number }[] = [];
    for (let i = 0; i < previewLines.length; i++) {
      const line = previewLines[i].toLowerCase();
      let pos = 0;
      while ((pos = line.indexOf(q, pos)) !== -1) {
        matches.push({ line: i, col: pos });
        pos += q.length;
      }
    }
    findMatches = matches;
    findIndex = matches.length > 0 ? 0 : -1;
    if (matches.length > 0) scrollToMatch(0);
  }

  function findNext() {
    if (findMatches.length === 0) return;
    findIndex = (findIndex + 1) % findMatches.length;
    scrollToMatch(findIndex);
  }

  function findPrev() {
    if (findMatches.length === 0) return;
    findIndex = (findIndex - 1 + findMatches.length) % findMatches.length;
    scrollToMatch(findIndex);
  }

  function scrollToMatch(idx: number) {
    const m = findMatches[idx];
    if (!m || !codeViewerRef) return;
    const lineEl = codeViewerRef.querySelector(`[data-line="${m.line}"]`);
    lineEl?.scrollIntoView({ block: "center", behavior: "smooth" });
  }

  function scrollToLine(lineNum: number) {
    if (!codeViewerRef || !previewLines) return;
    const clamped = Math.max(0, Math.min(lineNum - 1, previewLines.length - 1));
    const lineEl = codeViewerRef.querySelector(`[data-line="${clamped}"]`);
    lineEl?.scrollIntoView({ block: "center", behavior: "smooth" });
  }

  function handleGotoSubmit() {
    const num = parseInt(gotoLineValue, 10);
    if (!isNaN(num) && num > 0) {
      scrollToLine(num);
      closeGotoLine();
    }
  }

  function handlePreviewKeydown(event: KeyboardEvent) {
    const mod = event.metaKey || event.ctrlKey;
    if (mod && event.key === "f") {
      event.preventDefault();
      openFindBar();
    } else if (mod && event.key === "g") {
      event.preventDefault();
      openGotoLine();
    } else if (event.key === "Escape") {
      if (findBarOpen) closeFindBar();
      else if (gotoLineOpen) closeGotoLine();
      else closePreview();
    }
  }

  function isMatchLine(lineIdx: number): boolean {
    return findMatches.some((m) => m.line === lineIdx);
  }

  function isCurrentMatchLine(lineIdx: number): boolean {
    if (findIndex < 0 || findIndex >= findMatches.length) return false;
    return findMatches[findIndex].line === lineIdx;
  }

  function entryBadgeCounts(entry: WorkspaceEntry) {
    const conflictCount = entry.conflict_count ?? 0;
    const pendingDeleteCount = entry.pending_delete_count ?? 0;
    const dirtyCount = entry.dirty_count ?? 0;

    if (entry.is_directory) {
      return {
        conflictCount,
        dirtyCount,
        pendingDeleteCount
      };
    }

    if (entry.status === "conflicted") {
      return { conflictCount: 1, dirtyCount: 0, pendingDeleteCount: 0 };
    }

    if (entry.status === "deleted") {
      return { conflictCount: 0, dirtyCount: 0, pendingDeleteCount: 1 };
    }

    if (entry.status && entry.status !== "clean") {
      return { conflictCount: 0, dirtyCount: 1, pendingDeleteCount: 0 };
    }

    return {
      conflictCount,
      dirtyCount,
      pendingDeleteCount
    };
  }

  function diffKey(allowlistId: string, path: string) {
    return `${allowlistId}:${path}`;
  }

  function formatCheckpointTime(iso: string): string {
    const d = new Date(iso);
    const now = new Date();
    const diff = now.getTime() - d.getTime();
    if (diff < 60_000) return "刚刚";
    if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`;
    if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`;
    const month = d.getMonth() + 1;
    const day = d.getDate();
    const hh = String(d.getHours()).padStart(2, "0");
    const mm = String(d.getMinutes()).padStart(2, "0");
    return `${month}/${day} ${hh}:${mm}`;
  }

  let expandedCheckpoints = $state<Record<string, boolean>>({});
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
              {@const badges = entryBadgeCounts(entry)}
              <button
                class="tree-item {isEntryActive(entry) ? 'active' : ''}"
                onclick={() => onOpenEntry(entry)}
              >
                <span class="tree-item-icon {entry.is_directory ? 'folder' : ''}">
                  <Icon size={14} strokeWidth={2} />
                </span>
                <span class="tree-item-copy">
                  <span class="tree-item-name">{entryLabel(entry)}</span>
                  <span class="tree-item-meta">
                    {#if entryAllowlistId(entry)}
                      <span class="tree-item-subtle">{entryAllowlistId(entry)}</span>
                    {/if}
                  </span>
                </span>
                {#if badges.conflictCount || badges.dirtyCount || badges.pendingDeleteCount}
                  <span class="tree-item-badges">
                    {#if badges.conflictCount}
                      <span class="badge danger">{badges.conflictCount}</span>
                    {/if}
                    {#if badges.dirtyCount}
                      <span class="badge">{badges.dirtyCount}</span>
                    {/if}
                    {#if badges.pendingDeleteCount}
                      <span class="badge muted">{badges.pendingDeleteCount}</span>
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
                <div class="allowlist-header">
                  <strong class="allowlist-name">{group.allowlist.summary.allowlist.display_name}</strong>
                  <div class="allowlist-header-actions">
                    <button
                      class="header-icon-btn"
                      onclick={() => {
                        onCreateCheckpoint(
                          group.allowlist.summary.allowlist.id,
                          "手动存档",
                          "在侧栏手动创建"
                        );
                        showToast("正在创建存档点…");
                      }}
                      title="创建存档点"
                      aria-label="创建存档点"
                    >
                      <GitBranch size={14} strokeWidth={2} />
                    </button>
                    <button
                      class="header-icon-btn accent"
                      onclick={() => {
                        onKeepAllowlist(group.allowlist.summary.allowlist.id);
                        showToast("已保留全部变更", "success");
                      }}
                      title="全部保留"
                      aria-label="全部保留"
                    >
                      <Check size={14} strokeWidth={2.5} />
                    </button>
                    <button
                      class="header-icon-btn"
                      onclick={() => {
                        onRevertAllowlist(group.allowlist.summary.allowlist.id);
                        showToast("已撤销全部变更");
                      }}
                      title="全部撤销"
                      aria-label="全部撤销"
                    >
                      <Undo2 size={14} strokeWidth={2} />
                    </button>
                  </div>
                </div>

                <div class="file-change-list">
                  {#each group.entries as diff}
                    {@const stats = diffStats(diff.diff_text)}
                    <button
                      class="file-change-row"
                      type="button"
                      onclick={() => openDiffModal(group.allowlist.summary.allowlist.id, diff)}
                    >
                      <span class="file-status-badge {statusBadgeClass(diff.status)}">{statusBadge(diff.status)}</span>
                      <span class="file-name-col">
                        <span class="file-name">{fileName(diff.path)}</span>
                        {#if dirName(diff.path)}
                          <span class="file-dir">{dirName(diff.path)}</span>
                        {/if}
                      </span>
                      <span class="file-stats">
                        {#if stats.added > 0}<span class="stat-add">+{stats.added}</span>{/if}
                        {#if stats.removed > 0}<span class="stat-del">-{stats.removed}</span>{/if}
                      </span>
                    </button>
                  {/each}
                </div>

                {#if group.allowlist.checkpoints.length > 0}
                  {@const aid = group.allowlist.summary.allowlist.id}
                  {@const isExpanded = expandedCheckpoints[aid] ?? false}
                  <div class="checkpoint-section">
                    <button
                      class="checkpoint-toggle"
                      type="button"
                      onclick={() => expandedCheckpoints[aid] = !isExpanded}
                    >
                      <Clock size={12} strokeWidth={2} />
                      <span>存档点 ({group.allowlist.checkpoints.length})</span>
                      <ChevronRight size={12} strokeWidth={2} class="toggle-chevron {isExpanded ? 'expanded' : ''}" />
                    </button>
                    {#if isExpanded}
                      <div class="checkpoint-list" transition:fly={{ y: -8, duration: 180 }}>
                        {#each group.allowlist.checkpoints as cp}
                          <div class="checkpoint-item">
                            <div class="checkpoint-info">
                              <span class="checkpoint-label">
                                {#if cp.is_auto}
                                  <span class="checkpoint-auto-tag">自动</span>
                                {/if}
                                {cp.label ?? "未命名存档"}
                              </span>
                              <span class="checkpoint-meta">
                                <span class="checkpoint-time">{formatCheckpointTime(cp.created_at)}</span>
                                {#if cp.changed_files.length > 0}
                                  <span class="checkpoint-dot">·</span>
                                  <span class="checkpoint-files-count">{cp.changed_files.length} 个文件</span>
                                {/if}
                              </span>
                            </div>
                            <div class="checkpoint-actions">
                              <button
                                class="checkpoint-action-btn"
                                type="button"
                                title="恢复到此存档点"
                                onclick={() => {
                                  onRestoreCheckpoint(aid, cp.id);
                                  showToast("正在恢复到存档点…");
                                }}
                              >
                                <RotateCcw size={12} strokeWidth={2} />
                              </button>
                              <button
                                class="checkpoint-action-btn danger"
                                type="button"
                                title="删除此存档点"
                                onclick={() => {
                                  onDeleteCheckpoint(aid, cp.id);
                                }}
                              >
                                <Trash2 size={12} strokeWidth={2} />
                              </button>
                            </div>
                          </div>
                        {/each}
                      </div>
                    {/if}
                  </div>
                {/if}
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
      onkeydown={handlePreviewKeydown}
    >
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="preview-modal-inner" onclick={(event) => event.stopPropagation()}>
        <!-- Title bar -->
        <div class="preview-modal-head">
          <div class="preview-modal-title">
            <strong>{getPreviewPath() ?? "文件预览"}</strong>
            <span>
              {#if selectedDocument}
                工作区文件
              {:else if selectedFile}
                {statusLabel(selectedFile.status)}
              {:else}
                正在加载
              {/if}
              {#if previewLines}
                · {previewLines.length} 行
              {/if}
            </span>
          </div>
          <div class="preview-head-actions">
            {#if previewLines}
              <button class="preview-tool-btn" title="查找 (⌘F)" onclick={openFindBar}>
                <Search size={14} strokeWidth={2} />
              </button>
            {/if}
            <button class="icon-btn" onclick={closePreview} aria-label="关闭预览">
              <X size={16} strokeWidth={2} />
            </button>
          </div>
        </div>

        <!-- Find bar -->
        {#if findBarOpen}
          <div class="code-find-bar" transition:fly={{ y: -10, duration: 150 }}>
            <Search size={13} strokeWidth={2} class="find-icon" />
            <input
              bind:this={findInputRef}
              type="text"
              class="find-input"
              placeholder="查找…"
              bind:value={findQuery}
              oninput={performFind}
              onkeydown={(e) => {
                if (e.key === "Enter" && e.shiftKey) findPrev();
                else if (e.key === "Enter") findNext();
                else if (e.key === "Escape") closeFindBar();
              }}
            />
            <span class="find-count">
              {#if findQuery}
                {findMatches.length > 0 ? `${findIndex + 1}/${findMatches.length}` : "无结果"}
              {/if}
            </span>
            <button class="find-nav-btn" title="上一个 (Shift+Enter)" onclick={findPrev} disabled={findMatches.length === 0}>
              <ArrowUp size={13} strokeWidth={2} />
            </button>
            <button class="find-nav-btn" title="下一个 (Enter)" onclick={findNext} disabled={findMatches.length === 0}>
              <ArrowDown size={13} strokeWidth={2} />
            </button>
            <button class="find-nav-btn" title="关闭 (Esc)" onclick={closeFindBar}>
              <X size={13} strokeWidth={2} />
            </button>
          </div>
        {/if}

        <!-- Go-to-line bar -->
        {#if gotoLineOpen}
          <div class="code-find-bar" transition:fly={{ y: -10, duration: 150 }}>
            <span class="goto-label">跳转到行：</span>
            <input
              bind:this={gotoInputRef}
              type="number"
              class="find-input goto-input"
              min="1"
              max={previewLines?.length ?? 1}
              placeholder={`1–${previewLines?.length ?? "?"}`}
              bind:value={gotoLineValue}
              onkeydown={(e) => {
                if (e.key === "Enter") handleGotoSubmit();
                else if (e.key === "Escape") closeGotoLine();
              }}
            />
            <button class="find-nav-btn" title="跳转" onclick={handleGotoSubmit}>
              <Check size={13} strokeWidth={2} />
            </button>
            <button class="find-nav-btn" title="关闭 (Esc)" onclick={closeGotoLine}>
              <X size={13} strokeWidth={2} />
            </button>
          </div>
        {/if}

        <!-- Code body -->
        <div class="preview-modal-body">
          {#if fileLoading}
            <div class="preview-empty">正在加载文件预览…</div>
          {:else if highlightedLines}
            <div class="code-viewer" bind:this={codeViewerRef}>
              <table class="code-table">
                <tbody>
                  {#each highlightedLines as line, i}
                    <tr
                      class="code-line {isCurrentMatchLine(i) ? 'match-current' : isMatchLine(i) ? 'match-highlight' : ''}"
                      data-line={i}
                    >
                      <td class="line-number" style:--lnw="{lineNumberWidth}ch">{i + 1}</td>
                      <td class="line-content hljs">{@html line || " "}</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {:else if selectedFile}
            <p class="preview-meta">{previewHeadline(selectedFile)}</p>
            <div class="preview-summary">
              <p>{previewBody(selectedFile)}</p>
              {#if selectedFile.status !== "clean"}
                <button class="jump-button" onclick={jumpToChanges}>
                  跳到变更
                </button>
              {/if}
            </div>
          {:else}
            <div class="preview-empty">暂无可预览的内容</div>
          {/if}
        </div>

        <!-- Status bar -->
        {#if previewLines}
          <div class="preview-status-bar">
            <span>{detectLanguage(getPreviewPath())?.toUpperCase() ?? "纯文本"}</span>
            <span>{previewLines.length} 行</span>
            <span class="status-shortcut">⌘F 查找 · ⌘G 跳行 · Esc 关闭</span>
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

{#if diffModalFile}
  {@const dm = diffModalFile}
  {@const diff = dm.diff}
  {@const lines = parseDiffLines(diff.diff_text)}
  {@const stats = diffStats(diff.diff_text)}
  {@const key = diffKey(dm.allowlistId, diff.path)}
  <div
    class="diff-modal-backdrop"
    role="presentation"
    onclick={closeDiffModal}
    onkeydown={(e) => e.key === "Escape" && closeDiffModal()}
    transition:fade={{ duration: 150 }}
  >
    <div
      class="diff-modal"
      role="dialog"
      aria-modal="true"
      aria-label="文件差异对比"
      onclick={(e) => e.stopPropagation()}
      in:fly={{ y: 40, duration: 260 }}
      out:fly={{ y: 40, duration: 200 }}
    >
      <div class="diff-modal-head">
        <div class="diff-modal-title">
          <strong>{diff.path}</strong>
          <span class="diff-modal-meta">
            <span class="file-status-badge {statusBadgeClass(diff.status)}">{statusBadge(diff.status)}</span>
            {statusLabel(diff.status)}
            {#if stats.added > 0}<span class="stat-add">+{stats.added}</span>{/if}
            {#if stats.removed > 0}<span class="stat-del">-{stats.removed}</span>{/if}
          </span>
        </div>
        <div class="diff-modal-actions">
          {#if diff.status === "conflicted"}
            <button class="action-btn" onclick={() => { onResolveConflict(dm.allowlistId, diff.path, "keep_disk"); closeDiffModal(); showToast("已保留磁盘版本"); }}>
              保留磁盘版本
            </button>
            <button class="action-btn" onclick={() => { onResolveConflict(dm.allowlistId, diff.path, "keep_workspace"); closeDiffModal(); showToast("已保留工作区版本"); }}>
              保留工作区版本
            </button>
          {:else}
            <button class="action-btn compact-primary" onclick={() => { onKeepAllowlist(dm.allowlistId, diff.path); closeDiffModal(); showToast("已保留此文件", "success"); }}>
              <Check size={11} strokeWidth={2.5} />
              保留
            </button>
            <button class="action-btn" onclick={() => { onRevertAllowlist(dm.allowlistId, diff.path); closeDiffModal(); showToast("已撤销此文件"); }}>
              <Undo2 size={11} strokeWidth={2} />
              撤销
            </button>
          {/if}
          <button class="action-btn icon-only" onclick={closeDiffModal} aria-label="关闭">
            <X size={14} strokeWidth={2} />
          </button>
        </div>
      </div>

      <div class="diff-modal-body">
        {#if diff.is_binary}
          <div class="diff-empty-hint">二进制文件，无法显示差异对比</div>
        {:else if diff.status === "conflicted"}
          <div class="conflict-side-by-side">
            <div class="conflict-pane">
              <div class="pane-label">磁盘版本</div>
              <pre class="pane-content">{diff.remote_content ?? "(空)"}</pre>
            </div>
            <div class="conflict-pane">
              <div class="pane-label">工作区版本</div>
              <pre class="pane-content">{diff.working_content ?? "(空)"}</pre>
            </div>
          </div>

          {#if diff.conflict_reason}
            <div class="conflict-reason">{diff.conflict_reason}</div>
          {/if}

          <div class="merge-section">
            <div class="merge-section-head">
              <strong>手工合并</strong>
              <button class="action-btn" onclick={() => (mergeDrafts[key] = createMergeDraft(diff))}>重置</button>
            </div>
            <textarea class="merge-textarea" bind:value={mergeDrafts[key]} rows="8"></textarea>
            <div class="merge-section-foot">
              <button class="action-btn compact-primary" onclick={() => { onResolveConflict(dm.allowlistId, diff.path, "manual_merge", undefined, mergeDrafts[key]); closeDiffModal(); showToast("合并结果已保存", "success"); }}>
                <Save size={12} strokeWidth={2} />
                保存合并结果
              </button>
            </div>
          </div>
        {:else if lines.length > 0}
          <div class="diff-lines">
            {#each lines as line}
              <div class="diff-line diff-{line.type}">
                <span class="diff-line-marker">
                  {#if line.type === "add"}+{:else if line.type === "remove"}-{:else if line.type === "header"}@@{:else}&nbsp;{/if}
                </span>
                <span class="diff-line-content">{line.content || " "}</span>
              </div>
            {/each}
          </div>
        {:else if diff.working_content}
          <pre class="diff-full-content">{diff.working_content}</pre>
        {:else}
          <div class="diff-empty-hint">暂无差异内容</div>
        {/if}
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

  .search-box input {
    flex: 1;
    background: transparent;
    border: none;
    color: var(--text-primary);
    font: inherit;
    outline: none;
    font-size: 12px;
    line-height: 1.3;
  }

  .copy-row input,
  .manual-merge textarea {
    flex: 1;
    background: var(--bg-input);
    border: 1px solid var(--border-input);
    border-radius: 8px;
    padding: 6px 10px;
    color: var(--text-primary);
    font: inherit;
    font-size: 12px;
    outline: none;
    transition: border-color 0.15s;
  }

  .copy-row input:focus,
  .manual-merge textarea:focus {
    border-color: var(--accent-gold);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-gold) 14%, transparent);
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
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: 12px;
    line-height: 1.3;
  }

  .tree-item-copy {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
  }

  .tree-item-meta,
  .allowlist-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .tree-item-subtle,
  .allowlist-id {
    font-size: 10px;
    line-height: 1.2;
    color: var(--text-muted);
    font-family: ui-monospace, "SFMono-Regular", "SF Mono", Menlo, Monaco, Consolas, monospace;
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

  .diff-card pre,
  .conflict-columns pre {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    overflow: auto;
    font-size: 12px;
    line-height: 1.55;
    color: var(--text-primary);
    padding: 8px 10px;
    border-radius: 8px;
    background: var(--bg-input);
  }

  .jump-button,
  .action-btn,
  .checkpoint-pill {
    border-radius: 10px;
    padding: 6px 10px;
    background: var(--bg-input);
    color: var(--text-primary);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    font-size: 11px;
    font-weight: 600;
    border: 1px solid var(--border-input);
  }

  .action-btn.primary,
  .jump-button {
    background: var(--accent-gold);
    color: #fff;
    border-color: transparent;
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

  .allowlist-heading {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
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
    border-color: color-mix(in srgb, var(--accent-danger-text) 28%, var(--border-default));
  }

  .conflict-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    border-radius: 999px;
    background: var(--accent-danger);
    color: var(--accent-danger-text);
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
    border-color: color-mix(in srgb, var(--accent-danger-text) 20%, transparent);
    background: var(--accent-danger);
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
    background: rgba(0, 0, 0, 0.15);
    backdrop-filter: blur(10px);
  }

  .preview-modal {
    width: min(1100px, calc(100vw - 56px));
    height: min(82vh, 900px);
    border-radius: 18px;
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
    padding: 10px 16px;
    border-bottom: 1px solid var(--border-default);
    background: var(--bg-input);
  }

  .preview-modal-title {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 2px;
  }

  .preview-modal-title strong {
    color: var(--text-primary);
    font-size: 13px;
    font-weight: 600;
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .preview-modal-title span {
    color: var(--text-tertiary);
    font-size: 11px;
  }

  .preview-head-actions {
    display: flex;
    gap: 4px;
    align-items: center;
    flex-shrink: 0;
  }

  .preview-tool-btn {
    width: 28px;
    height: 28px;
    border-radius: 6px;
    border: 1px solid var(--border-input);
    background: transparent;
    color: var(--text-tertiary);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .preview-tool-btn:hover {
    background: var(--bg-elevated);
    color: var(--text-primary);
  }

  /* ── Find / Goto bar ── */
  .code-find-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 16px;
    border-bottom: 1px solid var(--border-default);
    background: var(--bg-input);
  }

  .code-find-bar :global(.find-icon) {
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .find-input {
    flex: 1;
    min-width: 80px;
    max-width: 260px;
    padding: 4px 8px;
    border: 1px solid var(--border-input);
    border-radius: 6px;
    background: var(--bg-surface);
    font-size: 12px;
    font-family: inherit;
    color: var(--text-primary);
    outline: none;
  }

  .find-input:focus {
    border-color: var(--accent-gold);
  }

  .goto-input {
    max-width: 100px;
  }

  .goto-label {
    font-size: 12px;
    color: var(--text-secondary);
    white-space: nowrap;
  }

  .find-count {
    font-size: 11px;
    color: var(--text-tertiary);
    white-space: nowrap;
    min-width: 48px;
    text-align: center;
  }

  .find-nav-btn {
    width: 24px;
    height: 24px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text-tertiary);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: all 0.1s;
  }

  .find-nav-btn:hover:not(:disabled) {
    background: var(--bg-elevated);
    color: var(--text-primary);
  }

  .find-nav-btn:disabled {
    opacity: 0.35;
    cursor: default;
  }

  /* ── Code viewer ── */
  .preview-modal-body {
    display: flex;
    flex: 1;
    min-height: 0;
    flex-direction: column;
    overflow: hidden;
    padding: 0;
  }

  .code-viewer {
    flex: 1;
    overflow: auto;
    background: var(--bg-surface);
    min-height: 0;
  }

  .code-table {
    border-collapse: collapse;
    width: 100%;
    min-height: 100%;
    font-family: ui-monospace, "SF Mono", Menlo, Monaco, Consolas, monospace;
    font-size: 12.5px;
    line-height: 1.55;
    tab-size: 4;
  }

  .code-line {
    transition: background 0.08s;
  }

  .code-line:hover {
    background: color-mix(in srgb, var(--accent-gold) 5%, transparent);
  }

  .code-line.match-highlight {
    background: color-mix(in srgb, var(--accent-gold) 12%, transparent);
  }

  .code-line.match-current {
    background: color-mix(in srgb, var(--accent-gold) 22%, transparent);
  }

  .line-number {
    position: sticky;
    left: 0;
    width: calc(var(--lnw, 3ch) + 32px);
    min-width: 48px;
    padding: 0 12px 0 16px;
    text-align: right;
    color: var(--text-tertiary);
    background: var(--bg-input);
    user-select: none;
    font-size: 11.5px;
    border-right: 1px solid var(--border-default);
    vertical-align: top;
  }

  .code-line:last-child .line-number,
  .code-line:last-child .line-content {
    height: 100%;
  }

  .line-content {
    padding: 0 16px;
    white-space: pre-wrap;
    word-break: break-all;
    min-height: 1.55em;
  }

  /* ── Status bar ── */
  .preview-status-bar {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 4px 16px;
    border-top: 1px solid var(--border-default);
    background: var(--bg-input);
    font-size: 11px;
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .status-shortcut {
    margin-left: auto;
    opacity: 0.7;
  }

  .preview-summary,
  .preview-empty {
    padding: 28px;
    text-align: center;
    color: var(--text-tertiary);
    font-size: 13px;
  }

  /* ── Compact file change list ── */
  .allowlist-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 4px 0;
  }

  .allowlist-name {
    font-size: 12px;
    font-weight: 700;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .allowlist-header-actions {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }

  .header-icon-btn {
    width: 30px;
    height: 28px;
    border-radius: 8px;
    border: 1px solid var(--border-input);
    background: var(--bg-input);
    color: var(--text-secondary);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .header-icon-btn:hover {
    background: var(--bg-elevated);
    color: var(--text-primary);
    border-color: var(--border-default);
  }

  .header-icon-btn.accent {
    background: var(--accent-gold);
    color: #fff;
    border-color: transparent;
  }

  .header-icon-btn.accent:hover {
    opacity: 0.88;
  }

  .file-change-list {
    display: flex;
    flex-direction: column;
  }

  .file-change-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    border: none;
    border-radius: 6px;
    background: transparent;
    cursor: pointer;
    font-family: inherit;
    width: 100%;
    text-align: left;
    transition: background 0.12s;
  }

  .file-change-row:hover {
    background: var(--bg-hover);
  }

  .file-status-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    border-radius: 4px;
    font-size: 10px;
    font-weight: 800;
    flex-shrink: 0;
    line-height: 1;
  }

  .badge-added {
    background: color-mix(in srgb, var(--accent-green) 18%, transparent);
    color: var(--accent-green);
  }

  .badge-modified {
    background: color-mix(in srgb, var(--accent-gold) 18%, transparent);
    color: var(--accent-gold);
  }

  .badge-deleted {
    background: var(--accent-danger);
    color: var(--accent-danger-text);
  }

  .badge-conflict {
    background: var(--accent-danger);
    color: var(--accent-danger-text);
  }

  .badge-other {
    background: var(--bg-hover);
    color: var(--text-tertiary);
  }

  .file-name-col {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 6px;
    overflow: hidden;
  }

  .file-name {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .file-dir {
    font-size: 10px;
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex-shrink: 1;
    min-width: 0;
  }

  .file-stats {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
    font-size: 11px;
    font-weight: 600;
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
  }

  .stat-add {
    color: var(--accent-green);
  }

  .stat-del {
    color: var(--accent-danger-text);
  }

  /* ── Checkpoint section ── */
  .checkpoint-section {
    margin-top: 4px;
    padding-top: 4px;
    border-top: 1px solid var(--border-default);
  }

  .checkpoint-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 8px;
    width: 100%;
    background: transparent;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    font-family: inherit;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-tertiary);
    transition: all 0.12s ease;
  }

  .checkpoint-toggle:hover {
    background: var(--bg-hover);
    color: var(--text-secondary);
  }

  .checkpoint-toggle :global(.toggle-chevron) {
    margin-left: auto;
    transition: transform 0.18s ease;
  }

  .checkpoint-toggle :global(.toggle-chevron.expanded) {
    transform: rotate(90deg);
  }

  .checkpoint-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 2px 0 4px;
  }

  .checkpoint-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px 5px 26px;
    border-radius: 6px;
    transition: background 0.1s ease;
  }

  .checkpoint-item:hover {
    background: var(--bg-hover);
  }

  .checkpoint-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .checkpoint-label {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .checkpoint-auto-tag {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-tertiary);
    background: var(--bg-elevated);
    padding: 1px 5px;
    border-radius: 4px;
    flex-shrink: 0;
  }

  .checkpoint-meta {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .checkpoint-time {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .checkpoint-dot {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .checkpoint-files-count {
    font-size: 11px;
    color: var(--text-tertiary);
    white-space: nowrap;
  }

  .checkpoint-actions {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
    opacity: 0;
    transition: opacity 0.12s ease;
  }

  .checkpoint-item:hover .checkpoint-actions {
    opacity: 1;
  }

  .checkpoint-action-btn {
    width: 24px;
    height: 24px;
    border-radius: 6px;
    border: 1px solid var(--border-input);
    background: var(--bg-input);
    color: var(--text-tertiary);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .checkpoint-action-btn:hover {
    background: var(--accent-gold);
    color: #fff;
    border-color: transparent;
  }

  .checkpoint-action-btn.danger:hover {
    background: var(--accent-danger);
    color: var(--accent-danger-text);
    border-color: transparent;
  }

  /* ── Diff modal ── */
  .diff-modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 70;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 28px;
    background: rgba(0, 0, 0, 0.15);
    backdrop-filter: blur(10px);
  }

  .diff-modal {
    width: min(900px, calc(100vw - 56px));
    max-height: min(82vh, 900px);
    display: flex;
    flex-direction: column;
    border-radius: 18px;
    border: 1px solid var(--border-default);
    background: var(--bg-surface);
    box-shadow: var(--shadow-dropdown);
    overflow: hidden;
  }

  .diff-modal-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 14px 18px;
    border-bottom: 1px solid var(--border-default);
  }

  .diff-modal-title {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .diff-modal-title strong {
    font-size: 14px;
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .diff-modal-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .diff-modal-actions {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }

  .diff-modal-body {
    flex: 1;
    overflow: auto;
    min-height: 0;
  }

  .diff-empty-hint {
    padding: 40px 20px;
    text-align: center;
    color: var(--text-muted);
    font-size: 13px;
  }

  /* ── Unified diff lines ── */
  .diff-lines {
    font-family: ui-monospace, "SF Mono", "Cascadia Code", Menlo, monospace;
    font-size: 12px;
    line-height: 1.6;
  }

  .diff-line {
    display: flex;
    padding: 0 14px;
    min-height: 22px;
  }

  .diff-line-marker {
    width: 20px;
    flex-shrink: 0;
    text-align: center;
    user-select: none;
    color: var(--text-muted);
  }

  .diff-line-content {
    flex: 1;
    white-space: pre-wrap;
    word-break: break-all;
    min-width: 0;
  }

  .diff-add {
    background: color-mix(in srgb, var(--accent-green) 12%, transparent);
  }

  .diff-add .diff-line-marker {
    color: var(--accent-green);
  }

  .diff-remove {
    background: color-mix(in srgb, var(--accent-danger-text) 10%, transparent);
  }

  .diff-remove .diff-line-marker {
    color: var(--accent-danger-text);
  }

  .diff-header {
    background: var(--bg-hover);
    color: var(--text-muted);
    font-size: 11px;
    padding: 4px 14px;
  }

  .diff-context {
    color: var(--text-secondary);
  }

  .diff-full-content {
    margin: 0;
    padding: 14px;
    font-size: 12px;
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--text-primary);
  }

  /* ── Conflict side-by-side ── */
  .conflict-side-by-side {
    display: grid;
    grid-template-columns: 1fr 1fr;
    min-height: 0;
  }

  .conflict-pane {
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--border-default);
    min-height: 0;
  }

  .conflict-pane:last-child {
    border-right: none;
  }

  .pane-label {
    padding: 8px 14px;
    font-size: 11px;
    font-weight: 700;
    color: var(--text-secondary);
    background: var(--bg-hover);
    border-bottom: 1px solid var(--border-default);
  }

  .pane-content {
    margin: 0;
    padding: 10px 14px;
    font-size: 12px;
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--text-primary);
    overflow: auto;
    flex: 1;
  }

  .conflict-reason {
    padding: 10px 14px;
    font-size: 12px;
    color: var(--accent-danger-text);
    background: var(--accent-danger);
    border-top: 1px solid var(--border-default);
  }

  .merge-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px;
    border-top: 1px solid var(--border-default);
  }

  .merge-section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .merge-section-head strong {
    font-size: 12px;
    color: var(--text-primary);
  }

  .merge-textarea {
    width: 100%;
    min-height: 120px;
    padding: 10px 12px;
    border: 1px solid var(--border-input);
    border-radius: 10px;
    background: var(--bg-input);
    color: var(--text-primary);
    font-size: 12px;
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
    line-height: 1.5;
    resize: vertical;
  }

  .merge-textarea:focus {
    outline: none;
    border-color: var(--accent-gold);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-gold) 14%, transparent);
  }

  .merge-section-foot {
    display: flex;
    justify-content: flex-end;
  }

  @media (max-width: 1100px) {
    .right-sidebar {
      width: 320px;
    }

    .conflict-side-by-side {
      grid-template-columns: 1fr;
    }

    .preview-modal,
    .diff-modal {
      width: calc(100vw - 24px);
      height: calc(100vh - 24px);
      border-radius: 18px;
    }

    .preview-modal-backdrop,
    .diff-modal-backdrop {
      padding: 12px;
    }
  }
</style>
