import { apiClient } from "../api";
import { notify } from "../tauri";
import { createEventStream, type StreamHandle } from "../stream";
import type {
  ActiveToolCall,
  SessionDetail,
  SessionMessage,
  SessionSummary,
  StreamEnvelope,
  StreamingState,
  TaskDetail,
  ToolDecision
} from "../types";

function emptyStreamingState(): StreamingState {
  return {
    streamingContent: "",
    thinking: false,
    thinkingMessage: "",
    toolCalls: [],
    reasoning: null,
    reasoningDecisions: [],
    suggestions: [],
    turnCost: null,
    images: [],
    isStreaming: false
  };
}

class SessionsState {
  list = $state<SessionSummary[]>([]);
  activeId = $state<string>("");
  active = $state<SessionDetail | null>(null);
  activeTaskDetail = $state<TaskDetail | null>(null);
  activeTaskLoading = $state(false);
  messageMode = $state<"ask" | "yolo">("ask");
  loading = $state(false);
  listLoading = $state(false);
  error = $state<string | null>(null);
  status = $state<string>("");
  streaming = $state<StreamingState>(emptyStreamingState());

  #streamHandle: StreamHandle | null = null;
  #pollTimer: ReturnType<typeof setTimeout> | null = null;

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
    this.streaming = emptyStreamingState();
    try {
      this.active = await apiClient.getSession(id);
      await this.refreshActiveTaskDetail();
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

  async delete(id: string) {
    this.error = null;
    try {
      await apiClient.deleteSession(id);
      if (this.activeId === id) {
        this.disconnect();
        this.active = null;
        this.activeId = "";
      }
      await this.fetchList();
      if (!this.activeId && this.list.length > 0) {
        await this.select(this.list[0].id);
      }
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to delete session";
    }
  }

  async sendMessage(content: string) {
    if (!content.trim() || !this.activeId || !this.active) return;

    // Clear any previous error when sending a new message
    this.error = null;

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

    // Reset streaming state for new message — immediately show thinking
    this.streaming = {
      ...emptyStreamingState(),
      isStreaming: true,
      thinking: true,
      thinkingMessage: "正在处理..."
    };

    try {
      const response = await apiClient.sendSessionMessage(
        this.activeId,
        content.trim(),
        this.messageMode
      );
      this.active = {
        ...this.active,
        current_task: response.task ?? this.active.current_task
      };
      await this.refreshActiveTaskDetail();
      this.status = response.task_id
        ? `Message queued in ${this.messageMode} mode`
        : "Message queued";

      // Fallback only if body streaming never arrives. Delay it enough that a
      // healthy SSE session is not replaced by a sudden full refresh.
      this.#startPollFallback();
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to send message";
      // Stop the thinking indicator on error
      this.streaming = {
        ...this.streaming,
        isStreaming: false,
        thinking: false,
        thinkingMessage: ""
      };
    }
  }

  disconnect() {
    this.#stopPollFallback();
    if (this.#streamHandle) {
      this.#streamHandle.close();
      this.#streamHandle = null;
    }
  }

  #startPollFallback() {
    this.#stopPollFallback();
    const sessionId = this.activeId;
    this.#pollTimer = setTimeout(async () => {
      try {
        if (
          !this.streaming.isStreaming ||
          this.activeId !== sessionId ||
          this.streaming.streamingContent.length > 0
        ) {
          return;
        }

        const fresh = await apiClient.getSession(sessionId);
        if (!fresh || this.activeId !== sessionId) return;

        // If DB has more messages than we're displaying, SSE missed them
        const currentCount = this.active?.messages.length ?? 0;
        if (fresh.messages.length > currentCount) {
          this.active = fresh;
          this.streaming = emptyStreamingState();
        }
      } catch {
        // Ignore poll errors
      } finally {
        this.#stopPollFallback();
      }
    }, 12_000);
  }

  #stopPollFallback() {
    if (this.#pollTimer) {
      clearTimeout(this.#pollTimer);
      this.#pollTimer = null;
    }
  }

  async refreshActiveTaskDetail() {
    const taskId = this.active?.current_task?.id;
    if (!taskId || !this.active) {
      this.activeTaskDetail = null;
      this.activeTaskLoading = false;
      return;
    }

    this.activeTaskLoading = true;
    try {
      const detail = await apiClient.getTask(taskId);
      this.activeTaskDetail = detail;
      this.active = {
        ...this.active,
        current_task: detail.task
      };
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to load active run detail";
    } finally {
      this.activeTaskLoading = false;
    }
  }

  #handleEvent(event: StreamEnvelope) {
    if (!this.active) return;

    switch (event.event) {
      case "session.stream_chunk": {
        const { content } = event.payload as { content: string };
        this.streaming = {
          ...this.streaming,
          streamingContent: this.streaming.streamingContent + content,
          isStreaming: true,
          thinking: false,
          thinkingMessage: ""
        };
        break;
      }

      case "session.response": {
        const { content } = event.payload as { content: string };
        this.#stopPollFallback();
        // Finalize: commit accumulated streaming content OR the response content as a message
        const finalContent = this.streaming.streamingContent || content;
        this.active = {
          ...this.active,
          messages: [
            ...this.active.messages,
            {
              id: crypto.randomUUID(),
              role: "assistant",
              content: finalContent,
              created_at: new Date().toISOString()
            }
          ]
        };
        // Keep tool calls, suggestions, turn cost, images for display but reset streaming text
        this.streaming = {
          ...this.streaming,
          streamingContent: "",
          isStreaming: false,
          thinking: false,
          thinkingMessage: ""
        };
        break;
      }

      case "session.thinking": {
        const { message } = event.payload as { message: string };
        this.streaming = {
          ...this.streaming,
          thinking: true,
          thinkingMessage: message,
          isStreaming: true
        };
        break;
      }

      case "session.tool_started": {
        const { name } = event.payload as { name: string };
        const newTool: ActiveToolCall = {
          id: crypto.randomUUID(),
          name,
          status: "running",
          startedAt: new Date().toISOString(),
          completedAt: null,
          error: null,
          parameters: null,
          resultPreview: null
        };
        this.streaming = {
          ...this.streaming,
          toolCalls: [...this.streaming.toolCalls, newTool],
          isStreaming: true,
          thinking: false,
          thinkingMessage: ""
        };
        break;
      }

      case "session.tool_completed": {
        const { name, success, error, parameters } = event.payload as {
          name: string;
          success: boolean;
          error?: string;
          parameters?: string;
        };
        const updatedCalls = [...this.streaming.toolCalls];
        // Find the last running instance of this tool
        for (let i = updatedCalls.length - 1; i >= 0; i--) {
          if (updatedCalls[i].name === name && updatedCalls[i].status === "running") {
            updatedCalls[i] = {
              ...updatedCalls[i],
              status: success ? "completed" : "failed",
              completedAt: new Date().toISOString(),
              error: error ?? null,
              parameters: parameters ?? null
            };
            break;
          }
        }
        this.streaming = { ...this.streaming, toolCalls: updatedCalls };
        break;
      }

      case "session.tool_result": {
        const { name, preview } = event.payload as { name: string; preview: string };
        const updatedCalls = [...this.streaming.toolCalls];
        // Attach preview to the most recent instance of this tool
        for (let i = updatedCalls.length - 1; i >= 0; i--) {
          if (updatedCalls[i].name === name) {
            updatedCalls[i] = { ...updatedCalls[i], resultPreview: preview };
            break;
          }
        }
        this.streaming = { ...this.streaming, toolCalls: updatedCalls };
        break;
      }

      case "session.reasoning_update": {
        const { narrative, decisions } = event.payload as {
          narrative: string;
          decisions: ToolDecision[];
        };
        this.streaming = {
          ...this.streaming,
          reasoning: narrative,
          reasoningDecisions: decisions ?? []
        };
        break;
      }

      case "session.suggestions": {
        const { suggestions } = event.payload as { suggestions: string[] };
        this.streaming = { ...this.streaming, suggestions: suggestions ?? [] };
        break;
      }

      case "session.turn_cost": {
        const payload = event.payload as { input_tokens: number; output_tokens: number; cost_usd: string };
        this.streaming = {
          ...this.streaming,
          turnCost: { input_tokens: payload.input_tokens, output_tokens: payload.output_tokens, cost_usd: payload.cost_usd }
        };
        break;
      }

      case "session.image_generated": {
        const { data_url, path } = event.payload as { data_url: string; path?: string };
        this.streaming = {
          ...this.streaming,
          images: [...this.streaming.images, { dataUrl: data_url, path: path ?? null }]
        };
        break;
      }

      case "session.approval_needed": {
        const { tool_name, summary } = event.payload as { tool_name: string; summary: string };
        this.status = `Approval needed: ${tool_name}`;
        void notify("Steward needs confirmation", `${tool_name}: ${summary}`);
        break;
      }

      case "session.error": {
        const { message } = event.payload as { message: string };
        this.error = message;
        this.streaming = {
          ...this.streaming,
          isStreaming: false,
          thinking: false
        };
        break;
      }

      case "session.status": {
        const { message } = event.payload as { message: string };
        this.status = message;
        break;
      }
    }

    void this.refreshActiveTaskDetail();
  }
}

export const sessionsStore = new SessionsState();
