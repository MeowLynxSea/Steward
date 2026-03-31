import { apiClient } from "../api";
import { notify } from "../tauri";
import { createEventStream, type StreamHandle } from "../stream";
import type { SessionDetail, SessionMessage, SessionSummary, RuntimeEvent } from "../types";

class SessionsState {
  list = $state<SessionSummary[]>([]);
  activeId = $state<string>("");
  active = $state<SessionDetail | null>(null);
  loading = $state(false);
  listLoading = $state(false);
  error = $state<string | null>(null);
  status = $state<string>("");

  #streamHandle: StreamHandle | null = null;

  async fetchList() {
    this.listLoading = true;
    this.error = null;
    try {
      const response = await apiClient.listSessions();
      this.list = response.sessions;
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to load sessions";
    } finally {
      this.listLoading = false;
    }
  }

  async select(id: string) {
    // Tear down any existing stream before switching.
    this.disconnect();

    this.activeId = id;
    this.loading = true;
    this.error = null;
    try {
      this.active = await apiClient.getSession(id);
      this.#streamHandle = createEventStream(
        `/sessions/${id}/stream`,
        this.#handleEvent.bind(this)
      );
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to load session";
    } finally {
      this.loading = false;
    }
  }

  async create(title = "New Session") {
    this.error = null;
    try {
      const created = await apiClient.createSession(title);
      await this.fetchList();
      await this.select(created.id);
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to create session";
    }
  }

  async sendMessage(content: string) {
    if (!content.trim() || !this.activeId || !this.active) return;

    const optimistic: SessionMessage = {
      id: crypto.randomUUID(),
      role: "user",
      content: content.trim(),
      created_at: new Date().toISOString()
    };
    this.active = {
      ...this.active,
      messages: [...this.active.messages, optimistic]
    };

    try {
      await apiClient.sendSessionMessage(this.activeId, content.trim());
      this.status = "Message queued";
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to send message";
    }
  }

  disconnect() {
    if (this.#streamHandle) {
      this.#streamHandle.close();
      this.#streamHandle = null;
    }
  }

  #handleEvent(event: RuntimeEvent) {
    if (!this.active) return;

    if (event.type === "response") {
      const { content } = event as { type: "response"; content: string; thread_id: string };
      this.active = {
        ...this.active,
        messages: [
          ...this.active.messages,
          {
            id: crypto.randomUUID(),
            role: "assistant",
            content,
            created_at: new Date().toISOString()
          }
        ]
      };
    } else if (event.type === "approval_needed") {
      const { tool_name, description } = event as { type: "approval_needed"; request_id: string; tool_name: string; description: string; parameters: string; thread_id?: string | null; allow_always: boolean };
      this.status = `Approval needed: ${tool_name}`;
      void notify("IronCowork needs confirmation", `${tool_name}: ${description}`);
    } else if (event.type === "error") {
      const { message } = event as { type: "error"; message: string; thread_id?: string | null };
      this.error = message;
    } else if (event.type === "status") {
      const { message } = event as { type: "status"; message: string; thread_id?: string | null };
      this.status = message;
    }
  }
}

export const sessionsStore = new SessionsState();
