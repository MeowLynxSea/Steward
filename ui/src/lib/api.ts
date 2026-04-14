import { invoke } from "@tauri-apps/api/core";
import type {
  PatchSettingsRequest,
  MemoryNodeDetail,
  MemorySidebarSection,
  MemoryTimelineEntry,
  ReflectionDetail,
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
  WorkspaceAllowlistCheckpoint,
  WorkspaceAllowlistDetail,
  WorkspaceAllowlistDiff,
  WorkspaceAllowlistFileView,
  WorkspaceAllowlistHistory,
  WorkspaceAllowlistSummary,
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

  getReflectionDetails(threadId: string, assistantMessageId: string) {
    return invoke<ReflectionDetail>("get_reflection_details", {
      thread_id: threadId,
      assistant_message_id: assistantMessageId
    });
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

  createWorkspaceAllowlist(path: string, display_name?: string, bypass_write = true) {
    return invoke<WorkspaceAllowlistSummary>("create_workspace_allowlist", {
      payload: {
        path,
        display_name: display_name ?? null,
        bypass_write
      }
    });
  },

  listWorkspaceAllowlists() {
    return invoke<{ allowlists: WorkspaceAllowlistSummary[] }>("list_workspace_allowlists");
  },

  getWorkspaceAllowlist(id: string) {
    return invoke<WorkspaceAllowlistDetail>("get_workspace_allowlist", { id });
  },

  getWorkspaceAllowlistFile(id: string, path: string) {
    return invoke<WorkspaceAllowlistFileView>("get_workspace_allowlist_file", { id, path });
  },

  getWorkspaceAllowlistDiff(
    id: string,
    options?: {
      scopePath?: string;
      from?: string;
      to?: string;
      includeContent?: boolean;
      maxFiles?: number;
    }
  ) {
    return invoke<WorkspaceAllowlistDiff>("get_workspace_allowlist_diff", {
      id,
      payload: {
        scope_path: options?.scopePath ?? null,
        from: options?.from ?? null,
        to: options?.to ?? null,
        include_content: options?.includeContent ?? true,
        max_files: options?.maxFiles ?? null
      }
    });
  },

  createWorkspaceCheckpoint(
    id: string,
    label?: string,
    summary?: string,
    revisionId?: string
  ) {
    return invoke<WorkspaceAllowlistCheckpoint>("create_workspace_checkpoint", {
      id,
      payload: {
        revision_id: revisionId ?? null,
        label: label ?? null,
        summary: summary ?? null,
        created_by: "user",
        is_auto: false
      }
    });
  },

  listWorkspaceAllowlistCheckpoints(id: string, limit?: number) {
    return invoke<WorkspaceAllowlistCheckpoint[]>("list_workspace_allowlist_checkpoints", {
      id,
      payload: {
        limit: limit ?? null
      }
    });
  },

  getWorkspaceAllowlistHistory(
    id: string,
    options?: {
      scopePath?: string;
      limit?: number;
      since?: string;
      includeCheckpoints?: boolean;
    }
  ) {
    return invoke<WorkspaceAllowlistHistory>("get_workspace_allowlist_history", {
      id,
      payload: {
        scope_path: options?.scopePath ?? null,
        limit: options?.limit ?? 20,
        since: options?.since ?? null,
        include_checkpoints: options?.includeCheckpoints ?? true
      }
    });
  },

  keepWorkspaceAllowlist(id: string, scopePath?: string, checkpointId?: string) {
    return invoke<WorkspaceAllowlistDetail>("keep_workspace_allowlist", {
      id,
      payload: {
        scope_path: scopePath ?? null,
        checkpoint_id: checkpointId ?? null,
        set_as_baseline: true
      }
    });
  },

  revertWorkspaceAllowlist(id: string, scopePath?: string, checkpointId?: string) {
    return invoke<WorkspaceAllowlistDetail>("revert_workspace_allowlist", {
      id,
      payload: {
        scope_path: scopePath ?? null,
        checkpoint_id: checkpointId ?? null,
        set_as_baseline: false
      }
    });
  },

  restoreWorkspaceAllowlist(
    id: string,
    target: string,
    options?: {
      scopePath?: string;
      setAsBaseline?: boolean;
      dryRun?: boolean;
      createCheckpointBeforeRestore?: boolean;
    }
  ) {
    return invoke<WorkspaceAllowlistDetail>("restore_workspace_allowlist", {
      id,
      payload: {
        target,
        scope_path: options?.scopePath ?? null,
        set_as_baseline: options?.setAsBaseline ?? false,
        dry_run: options?.dryRun ?? false,
        create_checkpoint_before_restore:
          options?.createCheckpointBeforeRestore ?? true,
        created_by: "user"
      }
    });
  },

  setWorkspaceAllowlistBaseline(id: string, target: string) {
    return invoke<WorkspaceAllowlistDetail>("set_workspace_allowlist_baseline", {
      id,
      payload: { target }
    });
  },

  refreshWorkspaceAllowlist(id: string, scopePath?: string) {
    return invoke<WorkspaceAllowlistDetail>("refresh_workspace_allowlist", {
      id,
      payload: {
        scope_path: scopePath ?? null,
        checkpoint_id: null,
        set_as_baseline: false
      }
    });
  },

  resolveWorkspaceAllowlistConflict(
    id: string,
    path: string,
    resolution: "keep_disk" | "keep_workspace" | "write_copy" | "manual_merge",
    renamedCopyPath?: string,
    mergedContent?: string
  ) {
    return invoke<WorkspaceAllowlistDetail>("resolve_workspace_allowlist_conflict", {
      id,
      payload: {
        path,
        resolution,
        renamed_copy_path: renamedCopyPath ?? null,
        merged_content: mergedContent ?? null
      }
    });
  }
};
