import { apiClient } from "../api";
import type {
  MountedFileDiff,
  WorkspaceChangeGroup,
  WorkspaceDocumentView,
  WorkspaceEntry,
  WorkspaceMountDetail,
  WorkspaceMountFileView,
  WorkspaceSearchResult
} from "../types";

function mountIdFromUri(uri: string): string | null {
  if (!uri.startsWith("workspace://")) {
    return null;
  }

  const remainder = uri.slice("workspace://".length);
  if (!remainder) {
    return null;
  }

  const [mountId] = remainder.split("/", 1);
  return mountId || null;
}

function mountDisplayNameFromPath(path: string): string {
  const segments = path.split(/[\\/]/).filter(Boolean);
  return segments.at(-1) ?? path;
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
  selectedMount = $state<WorkspaceMountDetail | null>(null);
  selectedFile = $state<WorkspaceMountFileView | null>(null);
  selectedDocument = $state<WorkspaceDocumentView | null>(null);
  mountDiff = $state<MountedFileDiff[]>([]);
  changeGroups = $state<WorkspaceChangeGroup[]>([]);
  loading = $state(false);
  fileLoading = $state(false);
  searchLoading = $state(false);
  error = $state<string | null>(null);
  status = $state("");
  busyAction = $state<string | null>(null);
  #previewRequestId = 0;

  async fetch(path = this.currentPath) {
    const previousPath = this.currentPath;
    const nextMountId = mountIdFromUri(path);
    this.loading = true;
    this.error = null;
    try {
      const response = await apiClient.getWorkspaceTree(path);
      this.currentPath = response.path;
      this.entries = response.entries;
      this.status = response.entries.length > 0 ? "工作区已就绪" : "这里还没有内容";
      try {
        await this.#refreshAllMountChanges();
      } catch (changesError) {
        this.error = errorMessage(changesError, "变更列表刷新失败");
      }
      if (previousPath !== response.path) {
        this.selectedFile = null;
        this.selectedDocument = null;
      }
      if (!nextMountId) {
        this.selectedMount = null;
        this.mountDiff = [];
      }
    } catch (e) {
      this.error = errorMessage(e, "Failed to load workspace");
    } finally {
      this.loading = false;
    }
  }

  async refresh() {
    const mountId =
      this.selectedMount?.summary.mount.id ?? mountIdFromUri(this.currentPath) ?? null;
    const selectedFilePath = this.selectedFile?.path ?? null;
    const selectedDocumentPath = this.selectedDocument?.path ?? null;

    this.error = null;
    await this.fetch(this.currentPath);

    if (mountId) {
      await this.#refreshMountState(mountId);
      if (selectedFilePath) {
        await this.#reloadSelectedFile(mountId, selectedFilePath);
      }
    }

    if (selectedDocumentPath && !mountIdFromUri(this.currentPath)) {
      await this.#reloadSelectedDocument(selectedDocumentPath);
    }
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

  async createMount(path: string, displayName?: string) {
    const trimmed = path.trim();
    if (!trimmed) {
      this.error = "Please enter a folder path to mount";
      return;
    }

    this.error = null;
    this.loading = true;
    try {
      const mount = await apiClient.createWorkspaceMount(
        trimmed,
        displayName?.trim() || mountDisplayNameFromPath(trimmed),
        true
      );
      this.clearSearch();
      this.selectedMount = null;
      this.selectedFile = null;
      this.selectedDocument = null;
      await this.fetch("workspace://");
      this.status = `已挂载 ${mount.mount.display_name}`;
    } catch (e) {
      this.error = errorMessage(e, "Failed to create mount");
    } finally {
      this.loading = false;
    }
  }

  async openEntry(entry: WorkspaceEntry) {
    this.error = null;
    const target = entry.uri ?? entry.path;
    const mountId = mountIdFromUri(target);

    if (mountId) {
      this.clearSearch();
      await this.loadMount(mountId);
      if (entry.is_directory) {
        this.selectedFile = null;
        this.selectedDocument = null;
        await this.fetch(target);
      } else {
        await this.loadFile(mountId, entry.path);
      }
      return;
    }

    if (entry.is_directory) {
      this.clearSearch();
      this.selectedMount = null;
      this.selectedFile = null;
      this.selectedDocument = null;
      await this.fetch(target);
      return;
    }

    await this.loadDocument(target);
  }

  async openPath(path: string) {
    this.clearSearch();
    const mountId = mountIdFromUri(path);
    if (mountId) {
      this.selectedDocument = null;
      this.selectedFile = null;
      await Promise.all([this.fetch(path), this.#refreshMountState(mountId)]);
      return;
    }

    this.selectedMount = null;
    this.selectedFile = null;
    this.selectedDocument = null;
    await this.fetch(path);
  }

  async loadMount(id: string) {
    this.error = null;
    this.selectedFile = this.selectedFile?.mount_id === id ? this.selectedFile : null;
    this.selectedDocument = null;
    await this.#refreshMountState(id);
  }

  async loadFile(id: string, path: string) {
    const requestId = ++this.#previewRequestId;
    this.fileLoading = true;
    this.error = null;
    this.selectedDocument = null;
    try {
      const file = await apiClient.getWorkspaceMountFile(id, path);
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
    this.selectedMount = null;
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

  async keepMount(id: string, scopePath?: string, checkpointId?: string) {
    await this.#runBusyAction("Saving workspace changes", async () => {
      this.selectedMount = await apiClient.keepWorkspaceMount(id, scopePath, checkpointId);
      await this.#afterMountMutation(id);
    });
  }

  async revertMount(id: string, scopePath?: string, checkpointId?: string) {
    await this.#runBusyAction("Reverting workspace changes", async () => {
      this.selectedMount = await apiClient.revertWorkspaceMount(id, scopePath, checkpointId);
      await this.#afterMountMutation(id);
    });
  }

  async createCheckpoint(id: string, label?: string, summary?: string) {
    await this.#runBusyAction("Creating checkpoint", async () => {
      await apiClient.createWorkspaceCheckpoint(id, label, summary);
      await this.#refreshMountState(id);
    });
  }

  async resolveConflict(
    id: string,
    path: string,
    resolution: "keep_disk" | "keep_workspace" | "write_copy" | "manual_merge",
    renamedCopyPath?: string,
    mergedContent?: string
  ) {
    await this.#runBusyAction("Resolving workspace conflict", async () => {
      this.selectedMount = await apiClient.resolveWorkspaceMountConflict(
        id,
        path,
        resolution,
        renamedCopyPath,
        mergedContent
      );
      await this.#afterMountMutation(id);
    });
  }

  dispose() {}

  async #afterMountMutation(id: string) {
    await Promise.all([
      this.fetch(this.currentPath),
      this.#refreshMountState(id),
      this.#refreshAllMountChanges()
    ]);

    if (this.selectedFile?.mount_id === id) {
      await this.#reloadSelectedFile(id, this.selectedFile.path);
    }
  }

  async #refreshMountState(id: string) {
    const [detail, diff] = await Promise.all([
      apiClient.getWorkspaceMount(id),
      apiClient.getWorkspaceMountDiff(id)
    ]);
    this.selectedMount = detail;
    this.mountDiff = diff.entries;
  }

  async #refreshAllMountChanges() {
    const mounts = await apiClient.listWorkspaceMounts();
    if (mounts.mounts.length === 0) {
      this.changeGroups = [];
      this.mountDiff = [];
      return;
    }

    const results = await Promise.all(
      mounts.mounts.map(async ({ mount }) => {
        const [detail, diff] = await Promise.all([
          apiClient.getWorkspaceMount(mount.id),
          apiClient.getWorkspaceMountDiff(mount.id)
        ]);
        return {
          ok: true as const,
          mount: detail,
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
      .filter((result): result is { ok: true; mount: WorkspaceMountDetail; entries: MountedFileDiff[] } => result.ok)
      .map((result) => ({
        mount: result.mount,
        entries: result.entries
      }));

    this.changeGroups = groups
      .filter((group) => group.entries.length > 0 || group.mount.checkpoints.length > 0)
      .sort((left, right) =>
        left.mount.summary.mount.display_name.localeCompare(right.mount.summary.mount.display_name)
      );

    if (this.selectedMount) {
      const updated = this.changeGroups.find(
        (group) => group.mount.summary.mount.id === this.selectedMount?.summary.mount.id
      );
      if (updated) {
        this.selectedMount = updated.mount;
        this.mountDiff = updated.entries;
      }
    }
  }

  async #reloadSelectedFile(id: string, path: string) {
    try {
      this.selectedFile = await apiClient.getWorkspaceMountFile(id, path);
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
    } catch (e) {
      this.error = errorMessage(e, "Workspace action failed");
    } finally {
      this.busyAction = null;
    }
  }
}

export const workspaceStore = new WorkspaceState();
