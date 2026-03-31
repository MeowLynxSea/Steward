export type TaskMode = "ask" | "yolo";
export type TaskStatus =
  | "idle"
  | "running"
  | "waiting_approval"
  | "completed"
  | "failed"
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
}

export interface TaskPendingOperation {
  request_id: string;
  tool_name: string;
  description: string;
  parameters: Record<string, unknown>;
  allow_always: boolean;
}

export interface TaskRecord {
  id: string;
  mode: TaskMode;
  status: TaskStatus;
  title: string;
  updated_at: string;
  pending_operation: TaskPendingOperation | null;
  last_error: string | null;
}

export interface WorkspaceEntry {
  path: string;
  is_directory: boolean;
  updated_at: string | null;
  content_preview: string | null;
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

export type RuntimeEvent =
  | { type: "response"; content: string; thread_id: string }
  | { type: "approval_needed"; request_id: string; tool_name: string; description: string; parameters: string; thread_id?: string | null; allow_always: boolean }
  | { type: "status"; message: string; thread_id?: string | null }
  | { type: "error"; message: string; thread_id?: string | null }
  | { type: string; [key: string]: unknown };
