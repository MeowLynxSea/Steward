export type TaskMode = "ask" | "yolo";
export type TaskStatus =
  | "queued"
  | "running"
  | "waiting_approval"
  | "completed"
  | "failed"
  | "cancelled"
  | "rejected";

export interface BackendInstance {
  id: string;
  provider: string;
  api_key: string | null;
  base_url: string | null;
  model: string;
  request_format: string | null;
}

export interface EmbeddingsSettings {
  enabled: boolean;
  provider: string;
  api_key: string | null;
  base_url: string | null;
  model: string;
  dimension: number | null;
}

export interface SettingsResponse {
  backends: BackendInstance[];
  major_backend_id: string | null;
  cheap_backend_id: string | null;
  cheap_model_uses_primary: boolean;
  embeddings: EmbeddingsSettings;
  llm_ready: boolean;
  llm_onboarding_required: boolean;
  llm_readiness_error: string | null;
}

export interface PatchSettingsRequest {
  backends?: BackendInstance[];
  major_backend_id?: string | null;
  cheap_backend_id?: string | null;
  cheap_model_uses_primary?: boolean;
  embeddings?: EmbeddingsSettings;
}

export interface SessionSummary {
  id: string;
  title: string;
  title_emoji: string | null;
  title_pending: boolean;
  turn_count: number;
  started_at: string;
  last_activity: string;
  active_thread_id: string | null;
}

export interface ThreadMessage {
  id: string;
  kind: "message" | "tool_call" | "thinking" | "reflection";
  role: string | null;
  content: string | null;
  created_at: string;
  turn_number: number;
  turn_cost: TurnCostInfo | null;
  tool_call: TimelineToolCall | null;
}

export type ReflectionStatus =
  | "queued"
  | "running"
  | "completed"
  | "failed"
  | "missing"
  | "unknown";

export type ReflectionOutcome =
  | "boot_promoted"
  | "updated"
  | "created"
  | "no_op"
  | "unknown";

export type ToolCallStatus = "running" | "completed" | "failed";

export interface TimelineToolCall {
  name: string;
  status: ToolCallStatus;
  startedAt?: string | null;
  completedAt?: string | null;
  parameters: string | null;
  resultPreview: string | null;
  error: string | null;
  rationale: string | null;
}

export interface ReflectionToolCall {
  id: string;
  created_at: string;
  tool_call: TimelineToolCall;
}

export interface ReflectionMessage {
  id: string;
  content: string;
  created_at: string;
}

export interface ReflectionDetail {
  assistant_message_id: string;
  status: ReflectionStatus;
  outcome: ReflectionOutcome | null;
  summary: string | null;
  detail: string | null;
  run_started_at: string | null;
  run_completed_at: string | null;
  tool_calls: ReflectionToolCall[];
  messages: ReflectionMessage[];
}

export interface SessionDetail {
  session: SessionSummary;
  active_thread_id: string;
  thread_messages: ThreadMessage[];
  active_thread_task: TaskRecord | null;
}

export interface TaskOperation {
  kind: string;
  tool_name: string;
  parameters: Record<string, unknown>;
  path: string | null;
  destination_path: string | null;
}

export interface TaskPendingApproval {
  id: string;
  risk: string;
  summary: string;
  operations: TaskOperation[];
  allow_always: boolean;
}

export interface TaskCurrentStep {
  id: string;
  kind: string;
  title: string;
}

export interface TaskRecord {
  id: string;
  template_id: string;
  mode: TaskMode;
  status: TaskStatus;
  title: string;
  created_at: string;
  updated_at: string;
  current_step: TaskCurrentStep | null;
  pending_approval: TaskPendingApproval | null;
  last_error: string | null;
  result_metadata: Record<string, unknown> | null;
}

export interface TaskTimelineEntry {
  sequence: number;
  event: string;
  status: TaskStatus;
  mode: TaskMode;
  current_step: TaskCurrentStep | null;
  pending_approval: TaskPendingApproval | null;
  last_error: string | null;
  result_metadata: Record<string, unknown> | null;
  created_at: string;
}

export interface TaskDetail {
  task: TaskRecord;
  timeline: TaskTimelineEntry[];
}

export interface WorkspaceEntry {
  path: string;
  uri?: string;
  name?: string;
  is_directory: boolean;
  updated_at: string | null;
  content_preview: string | null;
  kind?: string;
  status?: string | null;
  bypass_write?: boolean | null;
  dirty_count?: number;
  conflict_count?: number;
  pending_delete_count?: number;
}

export interface WorkspaceDocumentView {
  id: string;
  path: string;
  content: string;
  updated_at: string;
  created_at: string;
  metadata: Record<string, unknown> | null;
}

export interface WorkspaceSearchResult {
  document_id: string;
  document_path: string;
  chunk_id: string;
  content: string;
  score: number;
  fts_rank: number | null;
  vector_rank: number | null;
}

export type MemoryNodeKind =
  | "boot"
  | "identity"
  | "value"
  | "user_profile"
  | "directive"
  | "curated"
  | "episode"
  | "procedure"
  | "reference";

export interface MemoryRoute {
  id: string;
  space_id: string;
  edge_id: string | null;
  node_id: string;
  domain: string;
  path: string;
  is_primary: boolean;
  created_at: string;
  updated_at: string;
}

export interface MemoryVersion {
  id: string;
  node_id: string;
  supersedes_version_id: string | null;
  status: "active" | "deprecated" | "orphaned";
  content: string;
  metadata: Record<string, unknown> | null;
  created_at: string;
}

export interface MemoryKeyword {
  id: string;
  space_id: string;
  node_id: string;
  keyword: string;
  created_at: string;
}

export interface MemoryEdge {
  id: string;
  space_id: string;
  parent_node_id: string | null;
  child_node_id: string;
  relation_kind: string;
  visibility: string;
  priority: number;
  trigger_text: string | null;
  created_at: string;
  updated_at: string;
}

export interface MemorySearchHit {
  node_id: string;
  route_id: string;
  version_id: string;
  uri: string;
  title: string;
  kind: MemoryNodeKind;
  content_snippet: string;
  priority: number;
  trigger_text: string | null;
  score: number;
  fts_rank: number | null;
  vector_rank: number | null;
  is_hybrid_match?: boolean;
  matched_keywords?: string[];
  updated_at: string;
}

export interface MemoryNodeDetail {
  node: {
    id: string;
    space_id: string;
    kind: MemoryNodeKind;
    title: string;
    metadata: Record<string, unknown> | null;
    created_at: string;
    updated_at: string;
  };
  active_version: MemoryVersion;
  primary_route: MemoryRoute | null;
  selected_route: MemoryRoute | null;
  selected_uri: string | null;
  routes: MemoryRoute[];
  edges: MemoryEdge[];
  keywords: MemoryKeyword[];
  related_nodes: MemorySearchHit[];
}

export interface MemorySidebarItem {
  node_id: string;
  route_id: string | null;
  uri: string | null;
  title: string;
  subtitle: string | null;
  kind: MemoryNodeKind;
  updated_at: string;
}

export interface MemorySidebarSection {
  key: string;
  title: string;
  items: MemorySidebarItem[];
}

export interface MemoryTimelineEntry {
  node_id: string;
  route_id: string | null;
  uri: string | null;
  title: string;
  content_snippet: string;
  updated_at: string;
}

export interface MemoryChangeSet {
  id: string;
  space_id: string;
  origin: string;
  summary: string | null;
  status: string;
  created_at: string;
  updated_at: string;
}

export interface WorkspaceAllowlist {
  id: string;
  user_id: string;
  display_name: string;
  source_root?: string;
  bypass_read: boolean;
  bypass_write: boolean;
  created_at: string;
  updated_at: string;
}

export interface WorkspaceAllowlistSummary {
  allowlist: WorkspaceAllowlist;
  dirty_count: number;
  conflict_count: number;
  pending_delete_count: number;
}

export interface WorkspaceAllowlistCheckpoint {
  id: string;
  allowlist_id: string;
  revision_id: string;
  parent_checkpoint_id: string | null;
  label: string | null;
  summary: string | null;
  created_by: string;
  is_auto: boolean;
  base_generation: number;
  created_at: string;
  changed_files: string[];
}

export type AllowlistedFileStatus =
  | "clean"
  | "modified"
  | "added"
  | "deleted"
  | "conflicted"
  | "binary_modified";

export type WorkspaceAllowlistRevisionKind =
  | "initial"
  | "tool_write"
  | "tool_patch"
  | "tool_move"
  | "tool_delete"
  | "shell"
  | "fs_watch"
  | "manual_refresh"
  | "restore"
  | "accept";

export type WorkspaceAllowlistRevisionSource =
  | "workspace_tool"
  | "shell"
  | "external"
  | "system";

export type WorkspaceAllowlistChangeKind = "added" | "modified" | "deleted" | "moved";

export interface AllowlistedFileDiff {
  path: string;
  uri: string;
  status: AllowlistedFileStatus;
  change_kind: WorkspaceAllowlistChangeKind;
  is_binary: boolean;
  base_content: string | null;
  working_content: string | null;
  remote_content: string | null;
  diff_text: string | null;
  conflict_reason: string | null;
}

export interface WorkspaceAllowlistDiff {
  allowlist_id: string;
  from_revision_id: string | null;
  to_revision_id: string | null;
  entries: AllowlistedFileDiff[];
}

export interface WorkspaceAllowlistDetail {
  summary: WorkspaceAllowlistSummary;
  baseline_revision_id: string | null;
  head_revision_id: string | null;
  checkpoints: WorkspaceAllowlistCheckpoint[];
  open_change_count: number;
}

export interface WorkspaceAllowlistRevision {
  id: string;
  allowlist_id: string;
  parent_revision_id: string | null;
  kind: WorkspaceAllowlistRevisionKind;
  source: WorkspaceAllowlistRevisionSource;
  trigger: string | null;
  summary: string | null;
  created_by: string;
  created_at: string;
  changed_files: string[];
}

export interface WorkspaceAllowlistHistory {
  allowlist_id: string;
  baseline_revision_id: string | null;
  head_revision_id: string | null;
  revisions: WorkspaceAllowlistRevision[];
  checkpoints: WorkspaceAllowlistCheckpoint[];
}

export interface WorkspaceChangeGroup {
  allowlist: WorkspaceAllowlistDetail;
  entries: AllowlistedFileDiff[];
}

export interface WorkspaceAllowlistFileView {
  allowlist_id: string;
  path: string;
  uri: string;
  disk_path: string;
  status: AllowlistedFileStatus;
  is_binary: boolean;
  content: string | null;
  updated_at: string;
}

export interface WorkbenchMcpServer {
  name: string;
  transport: string;
  enabled: boolean;
  auth_mode: string;
  description: string | null;
}

export interface WorkbenchCapabilities {
  workspace_available: boolean;
  tool_count: number;
  dev_loaded_tools: string[];
  mcp_servers: WorkbenchMcpServer[];
}

export interface StreamEnvelope<T = Record<string, unknown>> {
  event: string;
  thread_id: string;
  sequence: number;
  timestamp: string;
  payload: T;
}

export interface SendSessionMessageResponse {
  accepted: boolean;
  session_id: string;
  active_thread_id: string;
  active_thread_task_id: string | null;
  active_thread_task: TaskRecord | null;
}

// --- Streaming state types ---

export interface ActiveToolCall extends TimelineToolCall {
  id: string;
  startedAt: string;
  completedAt: string | null;
}

export interface ToolDecision {
  tool_name: string;
  rationale: string;
}

export interface ReflectionStreamSignal {
  assistantMessageId: string;
  kind: "reflection" | "reflection_status" | "tool_started" | "tool_result" | "tool_completed";
  status?: ReflectionStatus;
  sequence: number;
}

export interface TurnCostInfo {
  input_tokens: number;
  output_tokens: number;
  cost_usd: string;
}

export interface StreamingState {
  /** Accumulated text from stream_chunk events, displayed with typewriter effect */
  streamingContent: string;
  /** Live assistant message being progressively revealed */
  assistantMessageId: string | null;
  /** Whether the agent is currently "thinking" */
  thinking: boolean;
  /** Persisted thinking message currently receiving chunks */
  thinkingMessageId: string | null;
  /** The thinking message text */
  thinkingMessage: string;
  /** Tool calls in progress or recently completed */
  toolCalls: ActiveToolCall[];
  /** Reasoning narrative from the agent */
  reasoning: string | null;
  /** Reasoning tool decisions */
  reasoningDecisions: ToolDecision[];
  /** Suggested follow-up messages */
  suggestions: string[];
  /** Token usage for the current turn */
  turnCost: TurnCostInfo | null;
  /** Generated images */
  images: Array<{ dataUrl: string; path: string | null }>;
  /** Latest reflection-related event tied to a specific assistant turn */
  reflectionSignal: ReflectionStreamSignal | null;
  /** Whether a response is actively streaming */
  isStreaming: boolean;
}

export interface SessionTitleUpdatePayload {
  session_id: string;
  title: string;
  emoji: string | null;
  pending: boolean;
}
