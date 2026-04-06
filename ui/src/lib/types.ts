export type TaskMode = "ask" | "yolo";
export type TaskStatus =
  | "queued"
  | "running"
  | "waiting_approval"
  | "completed"
  | "failed"
  | "cancelled"
  | "rejected";

export interface LlmBuiltinOverride {
  api_key: string | null;
  model: string | null;
  base_url: string | null;
  request_format: string | null;
}

export interface CustomLlmProviderSettings {
  id: string;
  name: string;
  adapter: string;
  base_url: string | null;
  default_model: string | null;
  api_key: string | null;
  builtin: boolean;
}

export interface SettingsResponse {
  llm_backend: string | null;
  selected_model: string | null;
  ollama_base_url: string | null;
  openai_compatible_base_url: string | null;
  llm_custom_providers: CustomLlmProviderSettings[];
  llm_builtin_overrides: Record<string, LlmBuiltinOverride>;
  llm_ready: boolean;
  llm_onboarding_required: boolean;
  llm_readiness_error: string | null;
}

export interface PatchSettingsRequest {
  llm_backend?: string | null;
  selected_model?: string | null;
  ollama_base_url?: string | null;
  openai_compatible_base_url?: string | null;
  llm_custom_providers?: CustomLlmProviderSettings[];
  llm_builtin_overrides?: Record<string, LlmBuiltinOverride>;
}

export interface SessionSummary {
  id: string;
  title: string;
  turn_count: number;
  started_at: string;
  last_activity: string;
  active_thread_id: string | null;
}

export interface ThreadMessage {
  id: string;
  kind: "message" | "tool_call";
  role: string | null;
  content: string | null;
  created_at: string;
  turn_number: number;
  tool_call: TimelineToolCall | null;
}

export type ToolCallStatus = "running" | "completed" | "failed";

export interface TimelineToolCall {
  name: string;
  status: ToolCallStatus;
  parameters: string | null;
  resultPreview: string | null;
  error: string | null;
  rationale: string | null;
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

export interface WorkspaceIndexJob {
  id: string;
  path: string;
  import_root: string;
  manifest_path: string;
  status: string;
  phase: string;
  total_files: number;
  processed_files: number;
  indexed_files: number;
  skipped_files: number;
  error: string | null;
  started_at: string;
  updated_at: string;
  completed_at: string | null;
}

export interface WorkspaceSearchResult {
  document_id: string;
  document_path: string;
  source_path: string | null;
  chunk_id: string;
  content: string;
  score: number;
  fts_rank: number | null;
  vector_rank: number | null;
}

export interface WorkspaceMount {
  id: string;
  user_id: string;
  display_name: string;
  source_root?: string;
  bypass_read: boolean;
  bypass_write: boolean;
  created_at: string;
  updated_at: string;
}

export interface WorkspaceMountSummary {
  mount: WorkspaceMount;
  dirty_count: number;
  conflict_count: number;
  pending_delete_count: number;
}

export interface WorkspaceMountCheckpoint {
  id: string;
  mount_id: string;
  parent_checkpoint_id: string | null;
  label: string | null;
  summary: string | null;
  created_by: string;
  is_auto: boolean;
  base_generation: number;
  created_at: string;
  changed_files: string[];
}

export interface MountedFileDiff {
  path: string;
  uri: string;
  status: string;
  is_binary: boolean;
  base_content: string | null;
  working_content: string | null;
  remote_content: string | null;
  diff_text: string | null;
  conflict_reason: string | null;
}

export interface WorkspaceMountDiff {
  mount_id: string;
  entries: MountedFileDiff[];
}

export interface WorkspaceMountDetail {
  summary: WorkspaceMountSummary;
  checkpoints: WorkspaceMountCheckpoint[];
  open_change_count: number;
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

export interface TurnCostInfo {
  input_tokens: number;
  output_tokens: number;
  cost_usd: string;
}

export interface StreamingState {
  /** Accumulated text from stream_chunk events, displayed with typewriter effect */
  streamingContent: string;
  /** Whether the agent is currently "thinking" */
  thinking: boolean;
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
  /** Whether a response is actively streaming */
  isStreaming: boolean;
}
