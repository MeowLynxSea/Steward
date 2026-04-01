export type TaskMode = "ask" | "yolo";
export type TaskStatus =
  | "queued"
  | "running"
  | "waiting_approval"
  | "completed"
  | "failed"
  | "cancelled"
  | "rejected";

export interface SettingsResponse {
  llm_backend: string | null;
  selected_model: string | null;
  ollama_base_url: string | null;
  openai_compatible_base_url: string | null;
  llm_custom_providers: Array<Record<string, unknown>>;
  llm_builtin_overrides: Record<string, Record<string, unknown>>;
}

export interface SessionSummary {
  id: string;
  title: string;
  message_count: number;
  started_at: string;
  last_activity: string;
  thread_type: string | null;
  channel: string;
}

export interface SessionMessage {
  id: string;
  role: string;
  content: string;
  created_at: string;
}

export interface SessionDetail {
  session: SessionSummary;
  messages: SessionMessage[];
  current_task: TaskRecord | null;
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
  is_directory: boolean;
  updated_at: string | null;
  content_preview: string | null;
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
  task_id: string | null;
  task: TaskRecord | null;
}
