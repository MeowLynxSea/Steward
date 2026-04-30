import { invoke } from "@tauri-apps/api/core";
import type {
  McpActivityListResponse,
  McpAddResourceToThreadResponse,
  McpAuthResponse,
  McpCompleteArgumentRequest,
  McpCompleteArgumentResponse,
  McpPromptGetRequest,
  McpPromptListResponse,
  McpPromptResponse,
  McpReadResourceResponse,
  McpSaveResourceSnapshotResponse,
  McpRespondElicitationRequest,
  McpRespondElicitationResponse,
  McpRespondSamplingRequest,
  McpRespondSamplingResponse,
  McpResourceListResponse,
  McpResourceTemplateListResponse,
  McpRootsResponse,
  McpServerListResponse,
  McpServerUpsertRequest,
  McpServerUpsertResponse,
  McpSetRootsRequest,
  McpTestResponse,
  McpToolListResponse,
  PatchSettingsRequest,
  MemoryChildEntry,
  MemoryNodeDetail,
  MemorySidebarSection,
  MemoryTimelineEntry,
  ReflectionDetail,
  MemoryChangeSet,
  MemoryVersion,
  MemorySearchHit,
  BrainWorkingMemoryResponse,
  BrainTopActivatedResponse,
  DroppedAttachmentFileResponse,
  SessionDetail,
  SessionRuntimeStatus,
  SendSessionMessageResponse,
  SendSessionMessageAttachmentRequest,
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

  getSessionRuntimeStatus(id: string) {
    return invoke<SessionRuntimeStatus>("get_session_runtime_status", { id });
  },

  interruptSession(id: string) {
    return invoke<SessionRuntimeStatus>("interrupt_session", { id });
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

  sendSessionMessage(
    id: string,
    content: string,
    attachments: SendSessionMessageAttachmentRequest[] = [],
    mode?: "ask" | "yolo"
  ) {
    return invoke<SendSessionMessageResponse>("send_session_message", {
      id,
      payload: {
        content,
        mode: mode ?? null,
        attachments
      }
    });
  },

  sheerSessionMessage(
    id: string,
    content: string,
    attachments: SendSessionMessageAttachmentRequest[] = [],
    mode?: "ask" | "yolo"
  ) {
    return invoke<SendSessionMessageResponse>("sheer_session_message", {
      id,
      payload: {
        content,
        mode: mode ?? null,
        attachments
      }
    });
  },

  queueSessionMessage(
    id: string,
    content: string,
    attachments: SendSessionMessageAttachmentRequest[] = [],
    mode?: "ask" | "yolo"
  ) {
    return invoke<SendSessionMessageResponse>("queue_session_message", {
      id,
      payload: {
        content,
        mode: mode ?? null,
        attachments
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
      payload: {
        approval_id: approvalId ?? null,
        always
      }
    });
  },

  rejectTask(id: string, approvalId?: string, reason?: string) {
    return invoke<TaskRecord>("reject_task", {
      id,
      payload: {
        approval_id: approvalId ?? null,
        reason: reason ?? null
      }
    });
  },

  cancelTask(id: string) {
    return invoke<TaskRecord>("cancel_task", { id });
  },

  patchTaskMode(id: string, mode: "ask" | "yolo") {
    return invoke<TaskRecord>("patch_task_mode", {
      id,
      payload: { mode }
    });
  },

  getWorkbenchCapabilities() {
    return invoke<WorkbenchCapabilities>("get_workbench_capabilities");
  },

  // -- MCP --

  listMcpServers() {
    return invoke<McpServerListResponse>("list_mcp_servers");
  },

  upsertMcpServer(payload: McpServerUpsertRequest) {
    return invoke<McpServerUpsertResponse>("upsert_mcp_server", { payload });
  },

  deleteMcpServer(name: string) {
    return invoke<string>("delete_mcp_server", { name });
  },

  testMcpServer(name: string) {
    return invoke<McpTestResponse>("test_mcp_server", { name });
  },

  beginMcpAuth(name: string) {
    return invoke<McpAuthResponse>("begin_mcp_auth", { name });
  },

  finishMcpAuth(name: string) {
    return invoke<McpAuthResponse>("finish_mcp_auth", { name });
  },

  listMcpTools(name: string) {
    return invoke<McpToolListResponse>("list_mcp_tools", { name });
  },

  listMcpResources(name: string) {
    return invoke<McpResourceListResponse>("list_mcp_resources", { name });
  },

  readMcpResource(name: string, uri: string) {
    return invoke<McpReadResourceResponse>("read_mcp_resource", { name, uri });
  },

  saveMcpResourceSnapshot(name: string, uri: string) {
    return invoke<McpSaveResourceSnapshotResponse>("save_mcp_resource_snapshot", { name, uri });
  },

  addMcpResourceToThreadContext(sessionId: string, name: string, uri: string) {
    return invoke<McpAddResourceToThreadResponse>("add_mcp_resource_to_thread_context", {
      session_id: sessionId,
      name,
      uri
    });
  },

  listMcpResourceTemplates(name: string) {
    return invoke<McpResourceTemplateListResponse>("list_mcp_resource_templates", { name });
  },

  subscribeMcpResource(name: string, uri: string) {
    return invoke<void>("subscribe_mcp_resource", { name, uri });
  },

  unsubscribeMcpResource(name: string, uri: string) {
    return invoke<void>("unsubscribe_mcp_resource", { name, uri });
  },

  listMcpPrompts(name: string) {
    return invoke<McpPromptListResponse>("list_mcp_prompts", { name });
  },

  getMcpPrompt(name: string, promptName: string, payload: McpPromptGetRequest) {
    return invoke<McpPromptResponse>("get_mcp_prompt", {
      name,
      prompt_name: promptName,
      payload
    });
  },

  completeMcpArgument(name: string, payload: McpCompleteArgumentRequest) {
    return invoke<McpCompleteArgumentResponse>("complete_mcp_argument", { name, payload });
  },

  getMcpRoots(name: string) {
    return invoke<McpRootsResponse>("get_mcp_roots", { name });
  },

  setMcpRoots(name: string, payload: McpSetRootsRequest) {
    return invoke<McpRootsResponse>("set_mcp_roots", { name, payload });
  },

  listMcpActivity() {
    return invoke<McpActivityListResponse>("list_mcp_activity");
  },

  respondMcpSampling(taskId: string, payload: McpRespondSamplingRequest) {
    return invoke<McpRespondSamplingResponse>("respond_mcp_sampling", {
      task_id: taskId,
      payload
    });
  },

  respondMcpElicitation(taskId: string, payload: McpRespondElicitationRequest) {
    return invoke<McpRespondElicitationResponse>("respond_mcp_elicitation", {
      task_id: taskId,
      payload
    });
  },

  readDroppedAttachmentFile(path: string) {
    return invoke<DroppedAttachmentFileResponse>("read_dropped_attachment_file", { path });
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

  listMemoryChildren(key: string) {
    return invoke<{ children: MemoryChildEntry[] }>("list_memory_children", { key });
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

  getBrainWorkingMemory(sessionId: string) {
    return invoke<BrainWorkingMemoryResponse>("get_brain_working_memory", { sessionId });
  },

  getBrainTopActivated(limit = 20) {
    return invoke<BrainTopActivatedResponse>("get_brain_top_activated", { limit });
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

  deleteWorkspaceAllowlist(id: string) {
    return invoke<void>("delete_workspace_allowlist", { id });
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

  deleteWorkspaceCheckpoint(allowlistId: string, checkpointId: string) {
    return invoke<void>("delete_workspace_checkpoint", {
      id: allowlistId,
      payload: {
        checkpoint_id: checkpointId
      }
    });
  },

  writeWorkspaceFile(path: string, content: string) {
    return invoke<void>("write_workspace_file", {
      payload: { path, content }
    });
  },

  deleteWorkspaceFile(path: string) {
    return invoke<void>("delete_workspace_file", {
      payload: { path }
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
