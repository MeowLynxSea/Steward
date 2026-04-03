import type {
  PatchSettingsRequest,
  SessionDetail,
  SendSessionMessageResponse,
  SessionSummary,
  SettingsResponse,
  TaskDetail,
  TaskRecord,
  WorkbenchCapabilities,
  WorkspaceEntry,
  WorkspaceIndexJob,
  WorkspaceMountCheckpoint,
  WorkspaceMountDetail,
  WorkspaceMountDiff,
  WorkspaceMountSummary,
  WorkspaceSearchResult
} from "./types";

const API_BASE = (globalThis as { __IRONCOWORK_API_BASE__?: string }).__IRONCOWORK_API_BASE__ ?? "/api/v0";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${API_BASE}${path}`, {
    headers: {
      "Content-Type": "application/json",
      ...(init?.headers ?? {})
    },
    ...init
  });

  if (!response.ok) {
    let message = `Request failed with ${response.status}`;
    try {
      const body = (await response.json()) as { error?: string };
      if (body.error) {
        message = body.error;
      }
    } catch {
      // Ignore body parse failures and keep the default message.
    }
    throw new Error(message);
  }

  if (response.status === 204) {
    return undefined as T;
  }

  return (await response.json()) as T;
}

export const apiClient = {
  // -- Settings --

  getSettings() {
    return request<SettingsResponse>("/settings");
  },

  patchSettings(payload: PatchSettingsRequest) {
    return request<SettingsResponse>("/settings", {
      method: "PATCH",
      body: JSON.stringify(payload)
    });
  },

  // -- Sessions --

  listSessions() {
    return request<{ sessions: SessionSummary[] }>("/sessions");
  },

  createSession(title?: string) {
    return request<{ id: string }>("/sessions", {
      method: "POST",
      body: JSON.stringify(title ? { title } : {})
    });
  },

  getSession(id: string) {
    return request<SessionDetail>(`/sessions/${id}`);
  },

  deleteSession(id: string) {
    return request<void>(`/sessions/${id}`, { method: "DELETE" });
  },

  sendSessionMessage(id: string, content: string, mode?: "ask" | "yolo") {
    return request<SendSessionMessageResponse>(`/sessions/${id}/messages`, {
      method: "POST",
      body: JSON.stringify({
        content,
        mode: mode ?? null
      })
    });
  },

  // -- Tasks --

  listTasks() {
    return request<{ tasks: TaskRecord[] }>("/tasks");
  },

  getTask(id: string) {
    return request<TaskDetail>(`/tasks/${id}`);
  },

  approveTask(id: string, approvalId?: string, always = false) {
    return request<TaskRecord>(`/tasks/${id}/approve`, {
      method: "POST",
      body: JSON.stringify({
        approval_id: approvalId ?? null,
        always
      })
    });
  },

  rejectTask(id: string, approvalId?: string, reason?: string) {
    return request<TaskRecord>(`/tasks/${id}/reject`, {
      method: "POST",
      body: JSON.stringify({
        approval_id: approvalId ?? null,
        reason: reason ?? null
      })
    });
  },

  cancelTask(id: string) {
    return request<TaskRecord>(`/tasks/${id}`, {
      method: "DELETE"
    });
  },

  patchTaskMode(id: string, mode: "ask" | "yolo") {
    return request<TaskRecord>(`/tasks/${id}/mode`, {
      method: "PATCH",
      body: JSON.stringify({ mode })
    });
  },

  getWorkbenchCapabilities() {
    return request<WorkbenchCapabilities>("/workbench/capabilities");
  },

  // -- Workspace --

  indexWorkspace(path: string) {
    return request<{ job: WorkspaceIndexJob }>("/workspace/index", {
      method: "POST",
      body: JSON.stringify({ path })
    });
  },

  getWorkspaceIndexJob(id: string) {
    return request<WorkspaceIndexJob>(`/workspace/index/${id}`);
  },

  getWorkspaceTree(path = "") {
    const query = path ? `?path=${encodeURIComponent(path)}` : "";
    return request<{ path: string; entries: WorkspaceEntry[] }>(`/workspace/tree${query}`);
  },

  searchWorkspace(query: string) {
    return request<{ results: WorkspaceSearchResult[] }>("/workspace/search", {
      method: "POST",
      body: JSON.stringify({ query })
    });
  },

  createWorkspaceMount(path: string, display_name?: string, bypass_write = true) {
    return request<WorkspaceMountSummary>("/workspace/mounts", {
      method: "POST",
      body: JSON.stringify({ path, display_name, bypass_write })
    });
  },

  listWorkspaceMounts() {
    return request<{ mounts: WorkspaceMountSummary[] }>("/workspace/mounts");
  },

  getWorkspaceMount(id: string) {
    return request<WorkspaceMountDetail>(`/workspace/mounts/${id}`);
  },

  getWorkspaceMountDiff(id: string, scopePath?: string) {
    const query = scopePath ? `?scope_path=${encodeURIComponent(scopePath)}` : "";
    return request<WorkspaceMountDiff>(`/workspace/mounts/${id}/diff${query}`);
  },

  createWorkspaceCheckpoint(id: string, label?: string, summary?: string) {
    return request<WorkspaceMountCheckpoint>(`/workspace/mounts/${id}/checkpoints`, {
      method: "POST",
      body: JSON.stringify({ label, summary, created_by: "user", is_auto: false })
    });
  },

  keepWorkspaceMount(id: string, scopePath?: string, checkpointId?: string) {
    return request<WorkspaceMountDetail>(`/workspace/mounts/${id}/keep`, {
      method: "POST",
      body: JSON.stringify({ scope_path: scopePath, checkpoint_id: checkpointId })
    });
  },

  revertWorkspaceMount(id: string, scopePath?: string, checkpointId?: string) {
    return request<WorkspaceMountDetail>(`/workspace/mounts/${id}/revert`, {
      method: "POST",
      body: JSON.stringify({ scope_path: scopePath, checkpoint_id: checkpointId })
    });
  },

  resolveWorkspaceMountConflict(
    id: string,
    path: string,
    resolution: "keep_disk" | "keep_workspace" | "write_copy" | "manual_merge",
    renamedCopyPath?: string,
    mergedContent?: string
  ) {
    return request<WorkspaceMountDetail>(`/workspace/mounts/${id}/resolve-conflict`, {
      method: "POST",
      body: JSON.stringify({
        path,
        resolution,
        renamed_copy_path: renamedCopyPath,
        merged_content: mergedContent
      })
    });
  }
};
