import { apiClient } from "../api";
import { notify } from "../tauri";
import { createEventStream, type StreamHandle } from "../stream";
import type {
  ActiveToolCall,
  SessionDetail,
  ThreadMessage,
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
  #streamingAssistantId: string | null = null;
  #liveTurnNumber: number | null = null;

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
    this.#streamingAssistantId = null;
    this.#liveTurnNumber = null;
    try {
      this.active = await apiClient.getSession(id);
      this.#liveTurnNumber = this.#inferLiveTurnNumber();
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

    const optimistic = {
      id: crypto.randomUUID(),
      kind: "message" as const,
      role: "user",
      content: content.trim(),
      created_at: new Date().toISOString(),
      turn_number: this.#nextTurnNumber(),
      tool_call: null
    };
    this.#liveTurnNumber = optimistic.turn_number;
    this.#streamingAssistantId = null;
    this.active = {
      ...this.active,
      thread_messages: [...this.active.thread_messages, optimistic]
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
        active_thread_id: response.active_thread_id,
        active_thread_task: response.active_thread_task ?? this.active.active_thread_task
      };
      await this.refreshActiveTaskDetail();
      this.status = response.active_thread_task_id
        ? `Message queued in ${this.messageMode} mode`
        : "Message queued";

      // Fallback only if live Tauri events never arrive. Delay it enough that a
      // healthy event stream is not replaced by a sudden full refresh.
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
    this.#streamingAssistantId = null;
    this.#liveTurnNumber = null;
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

        // If DB has more messages than we're displaying, the live event stream missed them
        const currentCount = this.active?.thread_messages.length ?? 0;
        if (fresh.thread_messages.length > currentCount) {
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
    const taskId = this.active?.active_thread_task?.id;
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
        active_thread_task: detail.task
      };
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to load active run detail";
    } finally {
      this.activeTaskLoading = false;
    }
  }

  #handleEvent(event: StreamEnvelope) {
    if (!this.active || !this.#eventMatchesActiveThread(event)) return;

    switch (event.event) {
      case "session.stream_chunk": {
        const { content } = event.payload as { content: string };
        this.#appendAssistantChunk(content);
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
        this.#finalizeAssistantMessage(content);
        this.streaming = {
          ...this.streaming,
          streamingContent: "",
          isStreaming: false,
          thinking: false,
          thinkingMessage: ""
        };
        this.#liveTurnNumber = null;
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
        this.#streamingAssistantId = null;
        const newTool: ActiveToolCall = {
          id: crypto.randomUUID(),
          name,
          status: "running",
          startedAt: new Date().toISOString(),
          completedAt: null,
          error: null,
          parameters: null,
          resultPreview: null,
          rationale: null
        };
        this.#appendToolCallMessage(newTool);
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
        this.#updateLatestToolCall(name, (tool) => ({
          ...tool,
          status: success ? "completed" : "failed",
          completedAt: new Date().toISOString(),
          error: error ?? null,
          parameters: parameters ?? null
        }));
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
        this.#updateLatestToolCall(name, (tool) => ({ ...tool, resultPreview: preview }));
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
        const { tool_name, summary, description } = event.payload as {
          tool_name: string;
          summary?: string;
          description?: string;
        };
        this.status = `Approval needed: ${tool_name}`;
        void notify("Steward needs confirmation", `${tool_name}: ${summary ?? description ?? ""}`);
        break;
      }

      case "session.error": {
        const { message } = event.payload as { message: string };
        this.error = message;
        this.#streamingAssistantId = null;
        this.#liveTurnNumber = null;
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
        if (message === "task.completed") {
          this.#streamingAssistantId = null;
          this.#liveTurnNumber = null;
        }
        break;
      }
    }

    void this.refreshActiveTaskDetail();
  }

  #eventMatchesActiveThread(event: StreamEnvelope) {
    const activeThreadId = this.active?.active_thread_id;
    if (!activeThreadId || !event.thread_id) {
      return true;
    }
    return event.thread_id === activeThreadId;
  }

  #nextTurnNumber() {
    const turns = this.active?.thread_messages.map((message) => message.turn_number) ?? [];
    return turns.length > 0 ? Math.max(...turns) + 1 : 0;
  }

  #inferLiveTurnNumber() {
    const messages = this.active?.thread_messages ?? [];
    if (messages.length === 0) {
      return null;
    }
    const lastMessage = messages[messages.length - 1];
    if (lastMessage.kind === "message" && lastMessage.role === "assistant") {
      return null;
    }
    return lastMessage.turn_number;
  }

  #ensureLiveTurnNumber() {
    if (this.#liveTurnNumber !== null) {
      return this.#liveTurnNumber;
    }
    const inferred = this.#inferLiveTurnNumber();
    if (inferred !== null) {
      this.#liveTurnNumber = inferred;
      return inferred;
    }
    const nextTurn = this.#nextTurnNumber();
    this.#liveTurnNumber = nextTurn;
    return nextTurn;
  }

  #appendMessage(entry: ThreadMessage) {
    if (!this.active) return;
    this.active = {
      ...this.active,
      thread_messages: [...this.active.thread_messages, entry]
    };
  }

  #updateMessage(id: string, mutate: (message: ThreadMessage) => ThreadMessage) {
    if (!this.active) return;
    this.active = {
      ...this.active,
      thread_messages: this.active.thread_messages.map((message) =>
        message.id === id ? mutate(message) : message
      )
    };
  }

  #appendAssistantChunk(content: string) {
    if (!this.active) return;
    const now = new Date().toISOString();
    const turnNumber = this.#ensureLiveTurnNumber();
    if (!this.#streamingAssistantId) {
      const id = crypto.randomUUID();
      this.#streamingAssistantId = id;
      this.#appendMessage({
        id,
        kind: "message",
        role: "assistant",
        content,
        created_at: now,
        turn_number: turnNumber,
        tool_call: null
      });
      return;
    }

    this.#updateMessage(this.#streamingAssistantId, (message) => ({
      ...message,
      content: `${message.content ?? ""}${content}`
    }));
  }

  #finalizeAssistantMessage(content: string) {
    if (!this.active) return;
    const finalContent = this.streaming.streamingContent || content;
    if (this.#streamingAssistantId) {
      this.#updateMessage(this.#streamingAssistantId, (message) => ({
        ...message,
        content: finalContent || message.content
      }));
    } else if (finalContent) {
      this.#appendMessage({
        id: crypto.randomUUID(),
        kind: "message",
        role: "assistant",
        content: finalContent,
        created_at: new Date().toISOString(),
        turn_number: this.#ensureLiveTurnNumber(),
        tool_call: null
      });
    }
    this.#streamingAssistantId = null;
  }

  #appendToolCallMessage(tool: ActiveToolCall) {
    this.#appendMessage({
      id: tool.id,
      kind: "tool_call",
      role: null,
      content: null,
      created_at: tool.startedAt,
      turn_number: this.#ensureLiveTurnNumber(),
      tool_call: tool
    });
  }

  #updateLatestToolCall(name: string, mutate: (tool: ActiveToolCall) => ActiveToolCall) {
    if (!this.active) return;
    const messages = [...this.active.thread_messages];
    let fallbackIndex = -1;
    for (let i = messages.length - 1; i >= 0; i--) {
      const entry = messages[i];
      if (entry.kind === "tool_call" && entry.tool_call?.name === name) {
        if (entry.tool_call.status === "running") {
          messages[i] = {
            ...entry,
            tool_call: mutate(entry.tool_call as ActiveToolCall)
          };
          this.active = { ...this.active, thread_messages: messages };
          return;
        }
        if (fallbackIndex === -1) {
          fallbackIndex = i;
        }
      }
    }

    if (fallbackIndex >= 0) {
      const entry = messages[fallbackIndex];
      messages[fallbackIndex] = {
        ...entry,
        tool_call: mutate(entry.tool_call as ActiveToolCall)
      };
      this.active = { ...this.active, thread_messages: messages };
    }
  }
}

export const sessionsStore = new SessionsState();
