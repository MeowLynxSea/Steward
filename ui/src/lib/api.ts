import type {
  RuntimeEvent,
  SessionDetail,
  SessionSummary,
  SettingsResponse,
  TaskRecord,
  WorkspaceEntry,
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
  getSettings() {
    return request<SettingsResponse>("/settings");
  },

  patchSettings(payload: Partial<SettingsResponse>) {
    return request<SettingsResponse>("/settings", {
      method: "PATCH",
      body: JSON.stringify(payload)
    });
  },

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

  sendSessionMessage(id: string, content: string) {
    return request<{ accepted: boolean; session_id: string }>(`/sessions/${id}/messages`, {
      method: "POST",
      body: JSON.stringify({ content })
    });
  },

  listTasks() {
    return request<{ tasks: TaskRecord[] }>("/tasks");
  },

  getTask(id: string) {
    return request<TaskRecord>(`/tasks/${id}`);
  },

  approveTask(id: string, requestId?: string, always = false) {
    return request<TaskRecord>(`/tasks/${id}/approve`, {
      method: "POST",
      body: JSON.stringify({ request_id: requestId, always })
    });
  },

  toggleTaskYolo(id: string, enabled: boolean) {
    return request<TaskRecord>(`/tasks/${id}/yolo-toggle`, {
      method: "POST",
      body: JSON.stringify({ enabled })
    });
  },

  indexWorkspace(path: string) {
    return request<{ path: string; document_path: string }>("/workspace/index", {
      method: "POST",
      body: JSON.stringify({ path })
    });
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

  streamEvents(path: string, onEvent: (event: RuntimeEvent) => void) {
    const source = new EventSource(`${API_BASE}${path}`);
    const handle = (event: MessageEvent<string>) => {
      const payload = JSON.parse(event.data) as RuntimeEvent;
      onEvent(payload);
    };

    source.addEventListener("response", handle as EventListener);
    source.addEventListener("approval_needed", handle as EventListener);
    source.addEventListener("status", handle as EventListener);
    source.addEventListener("error", handle as EventListener);

    return () => source.close();
  }
};
