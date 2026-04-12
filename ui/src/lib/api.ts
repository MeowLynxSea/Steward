import { invoke } from "@tauri-apps/api/core";
import type {
  PatchSettingsRequest,
  MemoryNodeDetail,
  MemorySidebarSection,
  MemoryTimelineEntry,
  MemoryChangeSet,
  MemoryVersion,
  MemorySearchHit,
  SessionDetail,
  SendSessionMessageResponse,
  SessionSummary,
  SettingsResponse,
  TaskDetail,
  TaskRecord,
  WorkbenchCapabilities,
  WorkspaceDocumentView,
  WorkspaceEntry,
  WorkspaceMountCheckpoint,
  WorkspaceMountDetail,
  WorkspaceMountDiff,
  WorkspaceMountFileView,
  WorkspaceMountSummary,
  WorkspaceSearchResult
} from "./types";

export const apiClient = {
  // -- Settings --

  getSettings() {
    return invoke<SettingsResponse>("get_settings");
  },

  patchSettings(payload: PatchSettingsRequest) {
    return invoke<SettingsResponse>("patch_settings", { payload });
  },

  // -- Sessions --

  listSessions() {
    return invoke<{ sessions: SessionSummary[] }>("list_sessions");
  },

  createSession(title?: string) {
    return invoke<{ id: string }>("create_session", {
      payload: {
        title: title ?? null
      }
    });
  },

  getSession(id: string) {
    return invoke<SessionDetail>("get_session", { id });
  },

  deleteSession(id: string) {
    return invoke<void>("delete_session", { id });
  },

  sendSessionMessage(id: string, content: string, mode?: "ask" | "yolo") {
    return invoke<SendSessionMessageResponse>("send_session_message", {
      id,
      payload: {
        content,
        mode: mode ?? null
      }
    });
  },

  // -- Tasks --

  listTasks() {
    return invoke<{ tasks: TaskRecord[] }>("list_tasks");
  },

  getTask(id: string) {
    return invoke<TaskDetail>("get_task", { id });
  },

  approveTask(id: string, approvalId?: string, always = false) {
    return invoke<TaskRecord>("approve_task", {
      id,
      approval_id: approvalId ?? null,
      always
    });
  },

  rejectTask(id: string, approvalId?: string, reason?: string) {
    return invoke<TaskRecord>("reject_task", {
      id,
      approval_id: approvalId ?? null,
      reason: reason ?? null
    });
  },

  cancelTask(id: string) {
    return invoke<TaskRecord>("cancel_task", { id });
  },

  patchTaskMode(id: string, mode: "ask" | "yolo") {
    return invoke<TaskRecord>("patch_task_mode", { id, mode });
  },

  getWorkbenchCapabilities() {
    return invoke<WorkbenchCapabilities>("get_workbench_capabilities");
  },

  // -- Workspace --

  getWorkspaceTree(path = "") {
    return invoke<{ path: string; entries: WorkspaceEntry[] }>("get_workspace_tree", {
      path
    });
  },

  getWorkspaceDocument(path: string) {
    return invoke<WorkspaceDocumentView>("get_workspace_document", { path });
  },

  searchWorkspace(query: string) {
    return invoke<{ results: WorkspaceSearchResult[] }>("search_workspace", {
      payload: { query }
    });
  },

  listMemorySidebar() {
    return invoke<{ sections: MemorySidebarSection[] }>("list_memory_sidebar");
  },

  getMemoryNode(key: string) {
    return invoke<{ detail: MemoryNodeDetail | null }>("get_memory_node", { key });
  },

  searchMemoryGraph(query: string, limit = 12, domains?: string[]) {
    return invoke<{ results: MemorySearchHit[] }>("search_memory_graph", {
      payload: { query, limit, domains: domains ?? null }
    });
  },

  listMemoryTimeline() {
    return invoke<{ entries: MemoryTimelineEntry[] }>("list_memory_timeline");
  },

  listMemoryReviews() {
    return invoke<{ reviews: MemoryChangeSet[] }>("list_memory_reviews");
  },

  applyMemoryReview(id: string, action: "accept" | "rollback") {
    return invoke<{ reviews: MemoryChangeSet[] }>("apply_memory_review", {
      id,
      payload: { action }
    });
  },

  rollbackMemoryChangeset(id: string) {
    return invoke<{ reviews: MemoryChangeSet[] }>("rollback_memory_changeset", { id });
  },

  getMemoryVersions(key: string) {
    return invoke<{ versions: MemoryVersion[] }>("get_memory_versions", { key });
  },

  createWorkspaceMount(path: string, display_name?: string, bypass_write = true) {
    return invoke<WorkspaceMountSummary>("create_workspace_mount", {
      payload: {
        path,
        display_name: display_name ?? null,
        bypass_write
      }
    });
  },

  listWorkspaceMounts() {
    return invoke<{ mounts: WorkspaceMountSummary[] }>("list_workspace_mounts");
  },

  getWorkspaceMount(id: string) {
    return invoke<WorkspaceMountDetail>("get_workspace_mount", { id });
  },

  getWorkspaceMountFile(id: string, path: string) {
    return invoke<WorkspaceMountFileView>("get_workspace_mount_file", { id, path });
  },

  getWorkspaceMountDiff(id: string, scopePath?: string) {
    return invoke<WorkspaceMountDiff>("get_workspace_mount_diff", {
      id,
      scope_path: scopePath ?? null
    });
  },

  createWorkspaceCheckpoint(id: string, label?: string, summary?: string) {
    return invoke<WorkspaceMountCheckpoint>("create_workspace_checkpoint", {
      id,
      label: label ?? null,
      summary: summary ?? null,
      created_by: "user",
      is_auto: false
    });
  },

  keepWorkspaceMount(id: string, scopePath?: string, checkpointId?: string) {
    return invoke<WorkspaceMountDetail>("keep_workspace_mount", {
      id,
      scope_path: scopePath ?? null,
      checkpoint_id: checkpointId ?? null
    });
  },

  revertWorkspaceMount(id: string, scopePath?: string, checkpointId?: string) {
    return invoke<WorkspaceMountDetail>("revert_workspace_mount", {
      id,
      scope_path: scopePath ?? null,
      checkpoint_id: checkpointId ?? null
    });
  },

  resolveWorkspaceMountConflict(
    id: string,
    path: string,
    resolution: "keep_disk" | "keep_workspace" | "write_copy" | "manual_merge",
    renamedCopyPath?: string,
    mergedContent?: string
  ) {
    return invoke<WorkspaceMountDetail>("resolve_workspace_mount_conflict", {
      id,
      path,
      resolution,
      renamed_copy_path: renamedCopyPath ?? null,
      merged_content: mergedContent ?? null
    });
  }
};
