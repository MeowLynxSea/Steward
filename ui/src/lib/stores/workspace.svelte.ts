import { apiClient } from "../api";
import { showToast } from "./toast.svelte";
import type {
  AllowlistedFileDiff,
  WorkspaceChangeGroup,
  WorkspaceDocumentView,
  WorkspaceEntry,
  WorkspaceAllowlistDetail,
  WorkspaceAllowlistFileView,
  WorkspaceSearchResult
} from "../types";

function allowlistIdFromUri(uri: string): string | null {
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

function isAllowlistRootUri(uri: string, allowlistId?: string): boolean {
  return Boolean(allowlistId) && uri === `workspace://${allowlistId}`;
}

function allowlistDisplayNameFromPath(path: string): string {
  const segments = path.split(/[\\/]/).filter(Boolean);
  return segments.at(-1) ?? path;
}

function supportsWorkspaceChanges(allowlist: Pick<WorkspaceAllowlistDetail["summary"]["allowlist"], "mount_kind">) {
  return allowlist.mount_kind !== "skills";
}

function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message) {
    return error.message;
  }
  if (typeof error === "string" && error.trim()) {
    return error;
  }
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof (error as { message?: unknown }).message === "string"
  ) {
    return (error as { message: string }).message;
  }
  return fallback;
}

class WorkspaceState {
  currentPath = $state("workspace://");
  entries = $state<WorkspaceEntry[]>([]);
  searchResults = $state<WorkspaceSearchResult[]>([]);
  searchQuery = $state("");
  selectedAllowlist = $state<WorkspaceAllowlistDetail | null>(null);
  selectedFile = $state<WorkspaceAllowlistFileView | null>(null);
  selectedDocument = $state<WorkspaceDocumentView | null>(null);
  allowlistDiff = $state<AllowlistedFileDiff[]>([]);
  changeGroups = $state<WorkspaceChangeGroup[]>([]);
  loading = $state(false);
  refreshing = $state(false);
  fileLoading = $state(false);
  searchLoading = $state(false);
  error = $state<string | null>(null);
  status = $state("");
  busyAction = $state<string | null>(null);
  #previewRequestId = 0;
  #allowlistChangesRefreshPromise: Promise<void> | null = null;

  async fetch(
    path = this.currentPath,
    options: {
      silent?: boolean;
    } = {}
  ) {
    const previousPath = this.currentPath;
    const nextAllowlistId = allowlistIdFromUri(path);
    const silent = options.silent ?? false;
    const showLoading = !silent || this.entries.length === 0 || previousPath !== path;
    if (showLoading) {
      this.loading = true;
    } else {
      this.refreshing = true;
    }
    this.error = null;
    try {
      const response = await apiClient.getWorkspaceTree(path);
      this.currentPath = response.path;
      this.entries = response.entries;
      this.status = response.entries.length > 0 ? "工作区已就绪" : "这里还没有内容";
      if (previousPath !== response.path) {
        this.selectedFile = null;
        this.selectedDocument = null;
      }
      if (!nextAllowlistId) {
        this.selectedAllowlist = null;
        this.allowlistDiff = [];
      }
    } catch (e) {
      this.error = errorMessage(e, "Failed to load workspace");
    } finally {
      this.loading = false;
      this.refreshing = false;
    }
  }

  async refresh() {
    if (this.loading || this.refreshing || this.busyAction) {
      return;
    }

    const allowlistId =
      this.selectedAllowlist?.summary.allowlist.id ?? allowlistIdFromUri(this.currentPath) ?? null;
    const selectedFilePath = this.selectedFile?.path ?? null;
    const selectedDocumentPath = this.selectedDocument?.path ?? null;

    this.error = null;
    await this.fetch(this.currentPath, { silent: true });

    if (allowlistId) {
      await this.#refreshAllowlistState(allowlistId);
      if (selectedFilePath) {
        await this.#reloadSelectedFile(allowlistId, selectedFilePath);
      }
    }

    if (selectedDocumentPath && !allowlistIdFromUri(this.currentPath)) {
      await this.#reloadSelectedDocument(selectedDocumentPath);
    }

    await this.#refreshAllAllowlistChangesInBackground();
  }

  async search(query: string) {
    const trimmed = query.trim();
    this.searchQuery = query;

    if (!trimmed) {
      this.searchResults = [];
      this.searchLoading = false;
      return;
    }

    this.searchLoading = true;
    this.error = null;
    try {
      const response = await apiClient.searchWorkspace(trimmed);
      this.searchResults = response.results;
    } catch (e) {
      this.error = errorMessage(e, "Workspace search failed");
    } finally {
      this.searchLoading = false;
    }
  }

  clearSearch() {
    this.searchQuery = "";
    this.searchResults = [];
    this.searchLoading = false;
  }

  clearPreview() {
    this.#previewRequestId += 1;
    this.fileLoading = false;
    this.selectedFile = null;
    this.selectedDocument = null;
  }

  async createAllowlist(path: string, displayName?: string) {
    const trimmed = path.trim();
    if (!trimmed) {
      this.error = "Please enter a folder path to allowlist";
      return;
    }

    this.error = null;
    this.loading = true;
    try {
      const allowlist = await apiClient.createWorkspaceAllowlist(
        trimmed,
        displayName?.trim() || allowlistDisplayNameFromPath(trimmed),
        true
      );
      this.clearSearch();
      this.selectedAllowlist = null;
      this.selectedFile = null;
      this.selectedDocument = null;
      await this.fetch("workspace://");
      this.status = `已授权 ${allowlist.allowlist.display_name}`;
    } catch (e) {
      this.error = errorMessage(e, "Failed to create allowlist");
    } finally {
      this.loading = false;
    }
  }

  async openEntry(entry: WorkspaceEntry) {
    this.error = null;
    const target = entry.uri ?? entry.path;
    const allowlistId = allowlistIdFromUri(target);

    if (entry.is_directory) {
      this.clearSearch();
      this.selectedFile = null;
      this.selectedDocument = null;
      await this.fetch(target);
      if (!allowlistId) {
        this.selectedAllowlist = null;
        this.allowlistDiff = [];
      }
      return;
    }

    if (allowlistId) {
      this.clearSearch();
      this.selectedDocument = null;
      await this.loadFile(allowlistId, entry.path);
      return;
    }

    await this.loadDocument(target);
  }

  async openPath(path: string) {
    this.clearSearch();
    const allowlistId = allowlistIdFromUri(path);
    if (allowlistId) {
      this.selectedDocument = null;
      this.selectedFile = null;
      await this.fetch(path);
      return;
    }

    this.selectedAllowlist = null;
    this.selectedFile = null;
    this.selectedDocument = null;
    await this.fetch(path);
  }

  async loadAllowlist(id: string) {
    this.error = null;
    this.selectedFile = this.selectedFile?.allowlist_id === id ? this.selectedFile : null;
    this.selectedDocument = null;
    await this.#refreshAllowlistState(id);
  }

  async refreshAllowlistChanges() {
    if (this.#allowlistChangesRefreshPromise) {
      await this.#allowlistChangesRefreshPromise;
      return;
    }
    await this.#refreshAllAllowlistChangesInBackground();
  }

  async loadFile(id: string, path: string) {
    const requestId = ++this.#previewRequestId;
    this.fileLoading = true;
    this.error = null;
    this.selectedDocument = null;
    try {
      const file = await apiClient.getWorkspaceAllowlistFile(id, path);
      if (requestId !== this.#previewRequestId) {
        return;
      }
      this.selectedFile = file;
      this.status = `Previewing ${path}`;
    } catch (e) {
      if (requestId === this.#previewRequestId) {
        this.error = errorMessage(e, "Failed to load file preview");
      }
    } finally {
      if (requestId === this.#previewRequestId) {
        this.fileLoading = false;
      }
    }
  }

  async loadDocument(path: string) {
    const requestId = ++this.#previewRequestId;
    this.fileLoading = true;
    this.error = null;
    this.selectedAllowlist = null;
    this.selectedFile = null;
    try {
      const document = await apiClient.getWorkspaceDocument(path);
      if (requestId !== this.#previewRequestId) {
        return;
      }
      this.selectedDocument = document;
      this.status = `Previewing ${path}`;
    } catch (e) {
      if (requestId === this.#previewRequestId) {
        this.error = errorMessage(e, "Failed to load workspace document");
      }
    } finally {
      if (requestId === this.#previewRequestId) {
        this.fileLoading = false;
      }
    }
  }

  async keepAllowlist(id: string, scopePath?: string, checkpointId?: string) {
    await this.#runBusyAction("正在保留变更…", async () => {
      this.selectedAllowlist = await apiClient.keepWorkspaceAllowlist(id, scopePath, checkpointId);
      await this.#afterAllowlistMutation(id);
    });
  }

  async revertAllowlist(id: string, scopePath?: string, checkpointId?: string) {
    await this.#runBusyAction("正在撤销变更…", async () => {
      this.selectedAllowlist = await apiClient.revertWorkspaceAllowlist(id, scopePath, checkpointId);
      await this.#afterAllowlistMutation(id);
    });
  }

  async createCheckpoint(id: string, label?: string, summary?: string) {
    await this.#runBusyAction("正在创建存档点…", async () => {
      await apiClient.createWorkspaceCheckpoint(id, label, summary);
      await this.#refreshAllowlistState(id);
    });
  }

  async restoreCheckpoint(allowlistId: string, checkpointId: string) {
    await this.#runBusyAction("正在恢复到存档点…", async () => {
      this.selectedAllowlist = await apiClient.restoreWorkspaceAllowlist(
        allowlistId,
        checkpointId,
        { createCheckpointBeforeRestore: true }
      );
      await this.#afterAllowlistMutation(allowlistId);
    });
  }

  async resolveConflict(
    id: string,
    path: string,
    resolution: "keep_disk" | "keep_workspace" | "write_copy" | "manual_merge",
    renamedCopyPath?: string,
    mergedContent?: string
  ) {
    await this.#runBusyAction("正在解决冲突…", async () => {
      this.selectedAllowlist = await apiClient.resolveWorkspaceAllowlistConflict(
        id,
        path,
        resolution,
        renamedCopyPath,
        mergedContent
      );
      await this.#afterAllowlistMutation(id);
    });
  }

  async deleteCheckpoint(allowlistId: string, checkpointId: string) {
    await this.#runBusyAction("正在删除存档点…", async () => {
      await apiClient.deleteWorkspaceCheckpoint(allowlistId, checkpointId);
      await this.#refreshAllowlistState(allowlistId);
    });
  }

  async writeFile(path: string, content: string) {
    await this.#runBusyAction("正在保存文件…", async () => {
      await apiClient.writeWorkspaceFile(path, content);
      await this.fetch(this.currentPath);
    });
  }

  async deleteFile(path: string, allowlistId?: string) {
    const deletingAllowlist = isAllowlistRootUri(path, allowlistId);
    await this.#runBusyAction(deletingAllowlist ? "正在取消授权…" : "正在删除文件…", async () => {
      if (deletingAllowlist && allowlistId) {
        const fallbackPath =
          this.currentPath === path || this.currentPath.startsWith(`${path}/`)
            ? "workspace://"
            : this.currentPath;

        if (this.selectedAllowlist?.summary.allowlist.id === allowlistId) {
          this.selectedAllowlist = null;
        }
        if (this.selectedFile?.allowlist_id === allowlistId) {
          this.selectedFile = null;
        }

        await apiClient.deleteWorkspaceAllowlist(allowlistId);
        await Promise.all([
          this.fetch(fallbackPath),
          this.#refreshAllAllowlistChangesInBackground()
        ]);
        this.status = "已取消工作区授权";
        return;
      }

      await apiClient.deleteWorkspaceFile(path);
      if (allowlistId) {
        await this.#afterAllowlistMutation(allowlistId);
      } else {
        await this.fetch(this.currentPath);
      }
    });
  }

  dispose() {}

  async #afterAllowlistMutation(id: string) {
    await Promise.all([
      this.fetch(this.currentPath),
      this.#refreshAllowlistState(id),
      this.#refreshAllAllowlistChangesInBackground()
    ]);

    if (this.selectedFile?.allowlist_id === id) {
      await this.#reloadSelectedFile(id, this.selectedFile.path);
    }
  }

  async #refreshAllowlistState(id: string) {
    const detail = await apiClient.getWorkspaceAllowlist(id);
    this.selectedAllowlist = detail;
    if (!supportsWorkspaceChanges(detail.summary.allowlist)) {
      this.allowlistDiff = [];
      return;
    }
    const diff = await apiClient.getWorkspaceAllowlistDiff(id);
    this.allowlistDiff = diff.entries;
  }

  async #refreshAllAllowlistChanges() {
    const allowlists = await apiClient.listWorkspaceAllowlists();
    if (allowlists.allowlists.length === 0) {
      this.changeGroups = [];
      this.allowlistDiff = [];
      return;
    }

    const trackedAllowlists = allowlists.allowlists.filter(({ allowlist }) =>
      supportsWorkspaceChanges(allowlist)
    );
    const results = await Promise.all(
      trackedAllowlists.map(async ({ allowlist }) => {
        const [detail, diff] = await Promise.all([
          apiClient.getWorkspaceAllowlist(allowlist.id),
          apiClient.getWorkspaceAllowlistDiff(allowlist.id)
        ]);
        return {
          ok: true as const,
          allowlist: detail,
          entries: diff.entries
        };
      }).map((promise) =>
        promise.catch((error) => ({
          ok: false as const,
          error
        }))
      )
    );

    const failedResult = results.find((result) => !result.ok);
    if (failedResult && !this.error) {
      this.error = errorMessage(failedResult.error, "变更列表刷新失败");
    }

    const groups = results
      .filter((result): result is { ok: true; allowlist: WorkspaceAllowlistDetail; entries: AllowlistedFileDiff[] } => result.ok)
      .map((result) => ({
        allowlist: result.allowlist,
        entries: result.entries
      }));

    this.changeGroups = groups
      .filter((group) => group.entries.length > 0 || group.allowlist.checkpoints.length > 0)
      .sort((left, right) =>
        left.allowlist.summary.allowlist.display_name.localeCompare(right.allowlist.summary.allowlist.display_name)
      );

    if (this.selectedAllowlist) {
      const updated = this.changeGroups.find(
        (group) => group.allowlist.summary.allowlist.id === this.selectedAllowlist?.summary.allowlist.id
      );
      if (updated) {
        this.selectedAllowlist = updated.allowlist;
        this.allowlistDiff = updated.entries;
      }
    }
  }

  #refreshAllAllowlistChangesInBackground() {
    if (this.#allowlistChangesRefreshPromise) {
      return this.#allowlistChangesRefreshPromise;
    }

    const refreshPromise = this.#refreshAllAllowlistChanges()
      .catch((error) => {
        this.error = errorMessage(error, "变更列表刷新失败");
      })
      .finally(() => {
        if (this.#allowlistChangesRefreshPromise === refreshPromise) {
          this.#allowlistChangesRefreshPromise = null;
        }
      });

    this.#allowlistChangesRefreshPromise = refreshPromise;
    return refreshPromise;
  }

  async #reloadSelectedFile(id: string, path: string) {
    try {
      this.selectedFile = await apiClient.getWorkspaceAllowlistFile(id, path);
    } catch {
      this.selectedFile = null;
    }
  }

  async #reloadSelectedDocument(path: string) {
    try {
      this.selectedDocument = await apiClient.getWorkspaceDocument(path);
    } catch {
      this.selectedDocument = null;
    }
  }

  async #runBusyAction(label: string, action: () => Promise<void>) {
    this.busyAction = label;
    this.error = null;
    try {
      await action();
      this.status = label;
      showToast(label.replace("正在", "已完成："), "success");
    } catch (e) {
      const msg = errorMessage(e, "操作失败");
      this.error = msg;
      showToast(msg, "error");
    } finally {
      this.busyAction = null;
    }
  }
}

export const workspaceStore = new WorkspaceState();
