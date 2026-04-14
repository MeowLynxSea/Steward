import { apiClient } from "../api";
import { notify } from "../tauri";
import { createEventStream, type StreamHandle } from "../stream";
import type {
  ActiveToolCall,
  ReflectionStatus,
  SessionDetail,
  SessionTitleUpdatePayload,
  ThreadMessage,
  SessionSummary,
  StreamEnvelope,
  StreamingState,
  TaskDetail,
  TurnCostInfo,
  ToolDecision
} from "../types";

function emptyStreamingState(): StreamingState {
  return {
    streamingContent: "",
    assistantMessageId: null,
    thinking: false,
    thinkingMessageId: null,
    thinkingMessage: "",
    toolCalls: [],
    reasoning: null,
    reasoningDecisions: [],
    suggestions: [],
    turnCost: null,
    images: [],
    reflectionSignal: null,
    isStreaming: false
  };
}

function turnCostFromResultMetadata(metadata: Record<string, unknown> | null | undefined): TurnCostInfo | null {
  if (!metadata || typeof metadata !== "object") {
    return null;
  }

  const candidate = metadata.turn_cost;
  if (!candidate || typeof candidate !== "object") {
    return null;
  }
  const turnCost = candidate as Record<string, unknown>;

  const input_tokens = turnCost.input_tokens;
  const output_tokens = turnCost.output_tokens;
  const cost_usd = turnCost.cost_usd;

  if (
    typeof input_tokens !== "number" ||
    typeof output_tokens !== "number" ||
    typeof cost_usd !== "string"
  ) {
    return null;
  }

  return { input_tokens, output_tokens, cost_usd };
}

function mergeStreamingChunk(existing: string, incoming: string): string {
  if (!existing) {
    return incoming;
  }
  if (!incoming) {
    return existing;
  }

  const boundaries: number[] = [];
  for (let i = 0; i < incoming.length; i++) {
    if ((incoming.codePointAt(i) ?? 0) > 0xffff) {
      boundaries.push(i);
      i += 1;
    } else {
      boundaries.push(i);
    }
  }
  boundaries.push(incoming.length);

  for (let i = boundaries.length - 1; i >= 0; i -= 1) {
    const overlap = boundaries[i];
    if (overlap === 0) {
      continue;
    }
    if (existing.endsWith(incoming.slice(0, overlap))) {
      return `${existing}${incoming.slice(overlap)}`;
    }
  }

  return `${existing}${incoming}`;
}

function isSessionTitleUpdatePayload(payload: unknown): payload is SessionTitleUpdatePayload {
  if (!payload || typeof payload !== "object") {
    return false;
  }

  const candidate = payload as Record<string, unknown>;
  return (
    typeof candidate.session_id === "string" &&
    typeof candidate.title === "string" &&
    (candidate.emoji === null || typeof candidate.emoji === "string") &&
    typeof candidate.pending === "boolean"
  );
}

function reflectionAssistantMessageId(
  payload: unknown
): string | null {
  if (!payload || typeof payload !== "object") {
    return null;
  }

  const candidate = payload as Record<string, unknown>;
  if (candidate.source !== "routine" || candidate.routine_name !== "memory_reflection") {
    return null;
  }

  return typeof candidate.assistant_message_id === "string"
    ? candidate.assistant_message_id
    : null;
}

function reflectionLifecycleStatus(payload: unknown): ReflectionStatus | null {
  if (!payload || typeof payload !== "object") {
    return null;
  }

  const candidate = payload as Record<string, unknown>;
  switch (candidate.status) {
    case "queued":
    case "running":
    case "completed":
    case "failed":
    case "missing":
    case "unknown":
      return candidate.status;
    default:
      return null;
  }
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
  #streamingThinkingId: string | null = null;
  #liveTurnNumber: number | null = null;
  #seenEventKeys = new Set<string>();

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
    this.#streamingThinkingId = null;
    this.#liveTurnNumber = null;
    this.#seenEventKeys.clear();
    try {
      this.active = await apiClient.getSession(id);
      await this.refreshActiveTaskDetail();
      this.#restoreStreamingAnchors();
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
      turn_cost: null,
      tool_call: null
    };
    this.#liveTurnNumber = optimistic.turn_number;
    this.#streamingAssistantId = null;
    this.#streamingThinkingId = null;
    this.active = {
      ...this.active,
      thread_messages: [...this.active.thread_messages, optimistic]
    };
    this.#applySessionTitleUpdate({
      session_id: this.activeId,
      title: this.active.session.title,
      emoji: this.active.session.title_emoji,
      pending: true
    });

    // Reset streaming state for new message — immediately show thinking
    this.streaming = {
      ...emptyStreamingState(),
      isStreaming: true
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
      this.#applySessionTitleUpdate({
        session_id: this.activeId,
        title: this.active.session.title,
        emoji: this.active.session.title_emoji,
        pending: false
      });
      // Stop the thinking indicator on error
      this.streaming = {
        ...this.streaming,
        isStreaming: false,
        thinking: false,
        thinkingMessageId: null,
        thinkingMessage: ""
      };
    }
  }

  disconnect() {
    this.#stopPollFallback();
    this.#streamingAssistantId = null;
    this.#streamingThinkingId = null;
    this.#liveTurnNumber = null;
    this.#seenEventKeys.clear();
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

  #finishStreamingFromTerminalState() {
    this.#stopPollFallback();
    this.#streamingThinkingId = null;
    this.#liveTurnNumber = null;
    this.streaming = {
      ...this.streaming,
      streamingContent: "",
      isStreaming: false,
      thinking: false,
      thinkingMessageId: null,
      thinkingMessage: this.streaming.thinkingMessage
    };
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
      const persistedTurnCost = turnCostFromResultMetadata(detail.task.result_metadata);
      if (persistedTurnCost) {
        this.#attachTurnCostToLatestAssistant(persistedTurnCost);
        this.streaming = {
          ...this.streaming,
          turnCost: persistedTurnCost
        };
      }
      if (["completed", "failed", "cancelled", "rejected"].includes(detail.task.status)) {
        this.#finishStreamingFromTerminalState();
      }
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to load active run detail";
    } finally {
      this.activeTaskLoading = false;
    }
  }

  #handleEvent(event: StreamEnvelope) {
    if (event.event === "session.title_updated") {
      if (isSessionTitleUpdatePayload(event.payload)) {
        this.#applySessionTitleUpdate(event.payload);
      }
      return;
    }

    if (!this.active || !this.#eventMatchesActiveThread(event)) return;

    const eventKey = `${event.thread_id}:${event.sequence}:${event.event}`;
    if (this.#seenEventKeys.has(eventKey)) {
      return;
    }
    this.#seenEventKeys.add(eventKey);
    if (this.#seenEventKeys.size > 2048) {
      const oldest = this.#seenEventKeys.values().next().value;
      if (oldest) {
        this.#seenEventKeys.delete(oldest);
      }
    }

    switch (event.event) {
      case "session.stream_chunk": {
        const { content } = event.payload as { content: string };
        this.#streamingThinkingId = null;
        this.#appendAssistantChunk(content);
        this.streaming = {
          ...this.streaming,
          streamingContent: this.streaming.streamingContent + content,
          assistantMessageId: this.streaming.assistantMessageId ?? this.#streamingAssistantId,
          isStreaming: true,
          thinking: false,
          thinkingMessageId: null,
          thinkingMessage: this.streaming.thinkingMessage
        };
        break;
      }

      case "session.response": {
        const { content } = event.payload as { content: string };
        this.#stopPollFallback();
        this.#streamingThinkingId = null;
        this.#finalizeAssistantMessage(content);
        this.#finishStreamingFromTerminalState();
        break;
      }

      case "session.reflection": {
        const { content } = event.payload as { content: string };
        const assistantMessageId = reflectionAssistantMessageId(event.payload);
        this.#stopPollFallback();
        this.#streamingThinkingId = null;
        this.#appendReflectionMessage(content);
        if (assistantMessageId) {
          this.streaming = {
            ...this.streaming,
            reflectionSignal: {
              assistantMessageId,
              kind: "reflection",
              sequence: event.sequence
            }
          };
        }
        break;
      }

      case "session.reflection_status": {
        const assistantMessageId = reflectionAssistantMessageId(event.payload);
        const status = reflectionLifecycleStatus(event.payload);
        if (assistantMessageId) {
          this.streaming = {
            ...this.streaming,
            reflectionSignal: {
              assistantMessageId,
              kind: "reflection_status",
              status: status ?? undefined,
              sequence: event.sequence
            }
          };
        }
        break;
      }

      case "session.thinking": {
        const { message, message_id } = event.payload as {
          message: string;
          message_id?: string | null;
        };
        this.#appendThinkingMessage(message, message_id ?? null);
        this.streaming = {
          ...this.streaming,
          thinking: true,
          thinkingMessageId: message_id ?? this.streaming.thinkingMessageId,
          thinkingMessage: mergeStreamingChunk(this.streaming.thinkingMessage, message),
          isStreaming: true
        };
        break;
      }

      case "session.tool_started": {
        const { name, tool_call_id, parameters } = event.payload as {
          name: string;
          tool_call_id?: string;
          parameters?: string;
        };
        const assistantMessageId = reflectionAssistantMessageId(event.payload);
        this.#streamingAssistantId = null;
        this.#streamingThinkingId = null;
        const newTool: ActiveToolCall = {
          id: tool_call_id ?? crypto.randomUUID(),
          name,
          status: "running",
          startedAt: new Date().toISOString(),
          completedAt: null,
          error: null,
          parameters: parameters ?? null,
          resultPreview: null,
          rationale: null
        };
        this.#appendToolCallMessage(newTool);
        this.streaming = {
          ...this.streaming,
          toolCalls: [...this.streaming.toolCalls, newTool],
          reflectionSignal: assistantMessageId
            ? {
                assistantMessageId,
                kind: "tool_started",
                sequence: event.sequence
              }
            : this.streaming.reflectionSignal,
          isStreaming: true,
          thinking: false,
          thinkingMessageId: null,
          thinkingMessage: this.streaming.thinkingMessage
        };
        break;
      }

      case "session.tool_completed": {
        const { name, tool_call_id, success, error, parameters } = event.payload as {
          name: string;
          tool_call_id?: string;
          success: boolean;
          error?: string;
          parameters?: string;
        };
        const assistantMessageId = reflectionAssistantMessageId(event.payload);
        const updatedCalls = [...this.streaming.toolCalls];
        const nextTool = (tool: ActiveToolCall): ActiveToolCall => ({
          ...tool,
          status: success ? "completed" : "failed",
          completedAt: new Date().toISOString(),
          error: error ?? null,
          parameters: parameters ?? tool.parameters
        });
        this.#updateStreamingToolCall(updatedCalls, tool_call_id, name, nextTool);
        this.#updateLatestToolCall(tool_call_id, name, nextTool);
        this.streaming = {
          ...this.streaming,
          toolCalls: updatedCalls,
          reflectionSignal: assistantMessageId
            ? {
                assistantMessageId,
                kind: "tool_completed",
                sequence: event.sequence
              }
            : this.streaming.reflectionSignal
        };
        break;
      }

      case "session.tool_result": {
        const { name, tool_call_id, preview } = event.payload as {
          name: string;
          tool_call_id?: string;
          preview: string;
        };
        const assistantMessageId = reflectionAssistantMessageId(event.payload);
        const updatedCalls = [...this.streaming.toolCalls];
        const nextTool = (tool: ActiveToolCall) => ({ ...tool, resultPreview: preview });
        this.#updateStreamingToolCall(updatedCalls, tool_call_id, name, nextTool);
        this.#updateLatestToolCall(tool_call_id, name, nextTool);
        this.streaming = {
          ...this.streaming,
          toolCalls: updatedCalls,
          reflectionSignal: assistantMessageId
            ? {
                assistantMessageId,
                kind: "tool_result",
                sequence: event.sequence
              }
            : this.streaming.reflectionSignal
        };
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
        const turnCost = {
          input_tokens: payload.input_tokens,
          output_tokens: payload.output_tokens,
          cost_usd: payload.cost_usd
        };
        this.#attachTurnCostToLatestAssistant(turnCost, this.#ensureLiveTurnNumber());
        this.streaming = {
          ...this.streaming,
          turnCost
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
          thinking: false,
          thinkingMessageId: null
        };
        break;
      }

      case "session.status": {
        const { message } = event.payload as { message: string };
        this.status = message;
        if (["Done", "Interrupted", "Rejected", "task.completed", "task.rejected", "task.cancelled", "task.failed"].includes(message)) {
          this.#finishStreamingFromTerminalState();
        }
        break;
      }
    }

    void this.refreshActiveTaskDetail();
  }

  #eventMatchesActiveThread(event: StreamEnvelope) {
    const activeThreadId = this.active?.active_thread_id;
    if (!activeThreadId) {
      return true;
    }
    if (!event.thread_id) {
      return false;
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
    if (this.#hasActiveTurn()) {
      return lastMessage.turn_number;
    }
    if (lastMessage.kind === "message" && lastMessage.role === "assistant") {
      return null;
    }
    return lastMessage.turn_number;
  }

  #hasActiveTurn() {
    const status = this.active?.active_thread_task?.status;
    return !!status && !["completed", "failed", "cancelled", "rejected"].includes(status);
  }

  #restoreStreamingAnchors() {
    this.#liveTurnNumber = this.#inferLiveTurnNumber();

    const lastMessage = this.active?.thread_messages.at(-1) ?? null;
    const isRunning = this.active?.active_thread_task?.status === "running";
    const persistedTurnCost = turnCostFromResultMetadata(this.active?.active_thread_task?.result_metadata);

    this.streaming = {
      ...this.streaming,
      isStreaming: isRunning,
      turnCost: persistedTurnCost ?? this.streaming.turnCost
    };

    if (!this.#hasActiveTurn() || !lastMessage) {
      return;
    }

    if (lastMessage.kind === "message" && lastMessage.role === "assistant") {
      this.#streamingAssistantId = lastMessage.id;
      this.streaming = {
        ...this.streaming,
        assistantMessageId: lastMessage.id
      };
      return;
    }

    if (lastMessage.kind === "thinking") {
      this.#streamingThinkingId = lastMessage.id;
      this.streaming = {
        ...this.streaming,
        thinking: true,
        thinkingMessageId: lastMessage.id,
        thinkingMessage: lastMessage.content ?? this.streaming.thinkingMessage
      };
    }
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

  #appendThinkingMessage(content: string, messageId: string | null) {
    if (!content) return;
    const now = new Date().toISOString();
    if (!messageId) {
      this.#streamingThinkingId = null;
      return;
    }

    if (!this.active) return;
    const turnNumber = this.#ensureLiveTurnNumber();

    if (!this.active.thread_messages.some((message) => message.id === messageId)) {
      this.#streamingThinkingId = messageId;
      this.#appendMessage({
        id: messageId,
        kind: "thinking",
        role: null,
        content,
        created_at: now,
        turn_number: turnNumber,
        turn_cost: null,
        tool_call: null
      });
      return;
    }

    this.#streamingThinkingId = messageId;
    this.#updateMessage(messageId, (message) => ({
      ...message,
      content: mergeStreamingChunk(message.content ?? "", content)
    }));
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

  #attachTurnCostToLatestAssistant(turnCost: TurnCostInfo, turnNumber?: number | null) {
    if (!this.active) return;

    const targetTurnNumber = turnNumber ?? this.#inferLiveTurnNumber();
    const messages = [...this.active.thread_messages];
    for (let i = messages.length - 1; i >= 0; i--) {
      const message = messages[i];
      if (
        message.kind === "message" &&
        message.role === "assistant" &&
        (targetTurnNumber === null || message.turn_number === targetTurnNumber)
      ) {
        messages[i] = { ...message, turn_cost: turnCost };
        this.active = { ...this.active, thread_messages: messages };
        return;
      }
    }
  }

  #appendAssistantChunk(content: string) {
    if (!this.active) return;
    const now = new Date().toISOString();
    const turnNumber = this.#ensureLiveTurnNumber();
    if (!this.#streamingAssistantId) {
      const id = crypto.randomUUID();
      this.#streamingAssistantId = id;
      this.streaming = {
        ...this.streaming,
        assistantMessageId: id
      };
      this.#appendMessage({
        id,
        kind: "message",
        role: "assistant",
        content,
        created_at: now,
        turn_number: turnNumber,
        turn_cost: null,
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
    const finalContent = content || this.streaming.streamingContent;
    const turnNumber = this.#ensureLiveTurnNumber();
    const assistantMessagesForTurn = this.active.thread_messages.filter(
      (message) =>
        message.kind === "message" &&
        message.role === "assistant" &&
        message.turn_number === turnNumber
    );
    const lastAssistantMessage = assistantMessagesForTurn.at(-1) ?? null;
    const hadStreamedChunks = this.streaming.streamingContent.trim().length > 0;

    if (this.#streamingAssistantId) {
      this.streaming = {
        ...this.streaming,
        assistantMessageId: this.#streamingAssistantId
      };

      if (finalContent) {
        this.#updateMessage(this.#streamingAssistantId, (message) => ({
          ...message,
          content: finalContent || message.content
        }));
      }
    } else if (finalContent) {
      if (hadStreamedChunks && lastAssistantMessage) {
        this.streaming = {
          ...this.streaming,
          assistantMessageId: lastAssistantMessage.id
        };
      } else if (lastAssistantMessage) {
        this.streaming = {
          ...this.streaming,
          assistantMessageId: lastAssistantMessage.id
        };
        this.#updateMessage(lastAssistantMessage.id, (message) => ({
          ...message,
          content: finalContent || message.content
        }));
      } else {
        const id = crypto.randomUUID();
        this.streaming = {
          ...this.streaming,
          assistantMessageId: id
        };
        this.#appendMessage({
          id,
          kind: "message",
          role: "assistant",
          content: finalContent,
          created_at: new Date().toISOString(),
          turn_number: turnNumber,
          turn_cost: null,
          tool_call: null
        });
      }
    }
    if (this.streaming.turnCost) {
      this.#attachTurnCostToLatestAssistant(this.streaming.turnCost, turnNumber);
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
      turn_cost: null,
      tool_call: tool
    });
  }

  #appendReflectionMessage(content: string) {
    if (!this.active || !content.trim()) return;
    this.#appendMessage({
      id: crypto.randomUUID(),
      kind: "reflection",
      role: null,
      content,
      created_at: new Date().toISOString(),
      turn_number: this.#ensureLiveTurnNumber(),
      turn_cost: null,
      tool_call: null
    });
  }

  #updateStreamingToolCall(
    tools: ActiveToolCall[],
    toolCallId: string | undefined,
    name: string,
    mutate: (tool: ActiveToolCall) => ActiveToolCall
  ) {
    for (let i = tools.length - 1; i >= 0; i--) {
      const tool = tools[i];
      if (toolCallId ? tool.id === toolCallId : tool.name === name) {
        tools[i] = mutate(tool);
        return;
      }
    }
  }

  #updateLatestToolCall(
    toolCallId: string | undefined,
    name: string,
    mutate: (tool: ActiveToolCall) => ActiveToolCall
  ) {
    if (!this.active) return;
    const messages = [...this.active.thread_messages];
    let fallbackIndex = -1;
    for (let i = messages.length - 1; i >= 0; i--) {
      const entry = messages[i];
      if (
        entry.kind === "tool_call" &&
        entry.tool_call?.name === name &&
        (!toolCallId || entry.id === toolCallId)
      ) {
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

    if (!toolCallId) {
      for (let i = messages.length - 1; i >= 0; i--) {
        const entry = messages[i];
        if (entry.kind === "tool_call" && entry.tool_call?.name === name) {
          messages[i] = {
            ...entry,
            tool_call: mutate(entry.tool_call as ActiveToolCall)
          };
          this.active = { ...this.active, thread_messages: messages };
          return;
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

  #applySessionTitleUpdate(update: SessionTitleUpdatePayload) {
    const apply = (session: SessionSummary): SessionSummary =>
      session.id === update.session_id
        ? {
            ...session,
            title: update.title,
            title_emoji: update.emoji,
            title_pending: update.pending
          }
        : session;

    this.list = this.list.map(apply);

    if (this.active?.session.id === update.session_id) {
      this.active = {
        ...this.active,
        session: apply(this.active.session)
      };
    }
  }
}

export const sessionsStore = new SessionsState();
