<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import {
    ArrowUp,
    ChevronRight,
    CornerDownRight,
    FileText,
    Loader,
    Shield,
    Square,
    Wrench,
    Zap,
    Image,
    Brain,
    Sparkles,
    Music4,
    Paperclip,
    X,
    Circle
  } from "lucide-svelte";
  import type {
    ContextStats,
    ReflectionDetail,
    ReflectionStatus,
    SessionDetail,
    SessionRuntimeStatus,
    ThreadMessage,
    ThreadMessageAttachment,
    StreamingState,
    TaskMode,
    TaskRecord,
    TimelineToolCall
  } from "../lib/types";
  import { apiClient } from "../lib/api";
  import { renderMarkdown } from "../lib/markdown";
  import { listenForFileDrops } from "../lib/tauri";
  import TaskApprovalCard from "./TaskApprovalCard.svelte";
  import { onDestroy, onMount } from "svelte";

  interface Props {
    session: SessionDetail | null;
    runtimeStatus: SessionRuntimeStatus | null;
    task: TaskRecord | null;
    messageMode: TaskMode;
    streaming: StreamingState;
    loading: boolean;
    emptyLayout?: boolean;
    noBackend?: boolean;
    composerSeed?: { id: string; content: string } | null;
    onSendMessage: (content: string, files: File[]) => Promise<boolean>;
    onSheerSendMessage: (content: string, files: File[]) => Promise<boolean>;
    onQueueSendMessage: (content: string, files: File[]) => Promise<boolean>;
    onInterruptSession: () => Promise<boolean>;
    onChangeMessageMode: (mode: TaskMode) => void;
    onSuggestionClick?: (suggestion: string) => void;
    onApproveTask: (task: TaskRecord) => void;
    onApproveTaskAlways: (task: TaskRecord) => void;
    onRejectTask: (task: TaskRecord, reason: string) => void;
  }

  type DisplayEntry =
    | { kind: "message"; message: ThreadMessage }
    | { kind: "auxiliary_group"; id: string; messages: ThreadMessage[] };

  type ReflectionPanelState = {
    loading: boolean;
    error: string | null;
    detail: ReflectionDetail | null;
  };

  type ReflectionTimelineEntry =
    | {
        id: string;
        kind: "thinking";
        createdAt: string;
        content: string;
      }
    | {
        id: string;
        kind: "tool";
        createdAt: string;
        toolCall: TimelineToolCall;
      }
    | {
        id: string;
        kind: "message";
        createdAt: string;
        content: string;
      };

  type ComposerAttachment = {
    id: string;
    file: File;
  };

  let {
    session,
    runtimeStatus,
    task,
    messageMode,
    streaming,
    loading,
    emptyLayout = false,
    composerSeed = null,
    onSendMessage,
    onSheerSendMessage,
    onQueueSendMessage,
    onInterruptSession,
    onChangeMessageMode,
    onSuggestionClick,
    onApproveTask,
    onApproveTaskAlways,
    onRejectTask,
    noBackend = false
  }: Props = $props();

  let draftMessage = $state("");
  let rejectReason = $state("Rejected by user");
  let inputBoxRef: HTMLDivElement | null = $state(null);
  let textareaRef: HTMLTextAreaElement | null = $state(null);
  let fileInputRef: HTMLInputElement | null = $state(null);
  let messageListRef: HTMLDivElement | null = $state(null);
  let composerAttachments = $state<ComposerAttachment[]>([]);
  let dragOverComposer = $state(false);
  let sendControlHovered = $state(false);
  let nativeFileDropBridgeActive = $state(false);
  let nativeDragInsideComposer = $state(false);
  let expandedToolCalls = $state<Set<string>>(new Set());
  let activeReflectionAssistantId = $state<string | null>(null);
  let reflectionPanels = $state<Record<string, ReflectionPanelState>>({});
  let imagesExpanded = $state(false);
  let showYoloRiskModal = $state(false);
  let showContextStatsModal = $state(false);
  let animatedAssistantId = $state<string | null>(null);
  let animatedAssistantText = $state("");
  let typingTimer: ReturnType<typeof setTimeout> | null = null;
  let animatedThinkingId = $state<string | null>(null);
  let animatedThinkingText = $state("");
  let thinkingTypingTimer: ReturnType<typeof setTimeout> | null = null;
  let settlingAuxiliarySummaries = $state<Set<string>>(new Set());
  let lastLiveThinkingId = $state<string | null>(null);
  let lastComposerSeedId = $state<string | null>(null);
  let lastReflectionThreadId = $state<string | null>(null);
  let lastReflectionSignalKey = $state<string | null>(null);

  const auxiliarySummaryTimers = new Map<string, ReturnType<typeof setTimeout>>();
  const reflectionPollTimers = new Map<string, ReturnType<typeof setTimeout>>();

  const hasStreamingContent = $derived(
    streaming.images.length > 0
  );
  const contextStats = $derived(session?.context_stats ?? null);
  $effect(() => {
    if (contextStats) {
      console.debug('[ChatArea] contextStats changed:', JSON.stringify(contextStats));
    }
  });
  const contextUsagePercent = $derived.by(() => {
    const stats = contextStats;
    const modelCtx = session?.model_context_length;
    if (!stats || !modelCtx) return 0;
    const total = modelCtx;
    const used = stats.system_prompt_tokens + stats.mcp_prompts_tokens + stats.skills_tokens + stats.messages_tokens + stats.tool_use_tokens + stats.compact_buffer_tokens;
    if (total === 0) return 0;
    return Math.round((used / total) * 100);
  });
  const showEmptyLayout = $derived(!loading && emptyLayout);
  const isYoloMode = $derived(messageMode === "yolo");
  const canSubmit = $derived(draftMessage.trim().length > 0 || composerAttachments.length > 0);
  const isBusySession = $derived(
    runtimeStatus?.thread_state === "processing" || runtimeStatus?.thread_state === "awaiting_approval"
  );
  const normalizedStreamingThinking = $derived.by(() => normalizeThinkingTranscript(streaming.thinkingMessage));
  const hasLiveStreamingSignal = $derived.by(() => {
    return Boolean(
      streaming.streamingContent.trim() ||
      normalizedStreamingThinking.trim() ||
      streaming.toolCalls.length > 0 ||
      streaming.reasoning ||
      streaming.images.length > 0
    );
  });
  const activeReflectionMessage = $derived.by(() => {
    if (!activeReflectionAssistantId) {
      return null;
    }
    return (
      session?.thread_messages.find(
        (message) =>
          message.id === activeReflectionAssistantId &&
          message.kind === "message" &&
          message.role === "assistant"
      ) ?? null
    );
  });
  const displayEntries = $derived.by<DisplayEntry[]>(() => {
    const messages = session?.thread_messages ?? [];
    const entries: DisplayEntry[] = [];
    let auxBuffer: ThreadMessage[] = [];

    const flushAux = () => {
      if (auxBuffer.length === 0) {
        return;
      }
      entries.push({
        kind: "auxiliary_group",
        id: `aux-group-${auxBuffer[0].id}`,
        messages: auxBuffer
      });
      auxBuffer = [];
    };

    for (const message of messages) {
      if (message.kind === "reflection") {
        continue;
      }
      if (
        message.kind === "thinking" ||
        (message.kind === "tool_call" && message.tool_call)
      ) {
        auxBuffer.push(message);
        continue;
      }

      flushAux();
      entries.push({ kind: "message", message });
    }

    flushAux();
    return entries;
  });
  const observedReflectionTimelines = $derived.by<Map<string, ReflectionTimelineEntry[]>>(() => {
    const messages = session?.thread_messages ?? [];
    const processes = new Map<string, ReflectionTimelineEntry[]>();
    const assistantIndexes = messages
      .map((message, index) => ({ message, index }))
      .filter(
        ({ message }) => message.kind === "message" && message.role === "assistant"
      );

    for (const { message, index } of assistantIndexes) {
      const entries: ReflectionTimelineEntry[] = [];
      for (let cursor = index + 1; cursor < messages.length; cursor += 1) {
        const candidate = messages[cursor];
        if (
          candidate.kind === "message" &&
          (candidate.role === "user" || candidate.role === "assistant")
        ) {
          break;
        }
        if (candidate.kind === "thinking" && (candidate.content ?? "").trim()) {
          entries.push({
            id: candidate.id,
            kind: "thinking",
            createdAt: candidate.created_at,
            content: candidate.content ?? ""
          });
          continue;
        }
        if (candidate.kind === "tool_call" && candidate.tool_call) {
          entries.push({
            id: candidate.id,
            kind: "tool",
            createdAt: candidate.created_at,
            toolCall: candidate.tool_call
          });
          continue;
        }
        if (candidate.kind === "reflection" && (candidate.content ?? "").trim()) {
          entries.push({
            id: candidate.id,
            kind: "message",
            createdAt: candidate.created_at,
            content: candidate.content ?? ""
          });
        }
      }

      processes.set(message.id, entries);
    }

    return processes;
  });
  const reasoningSummary = $derived(buildCompactSummary(streaming.reasoning ?? "", 110));
  const imageSummary = $derived(streaming.images.length > 0 ? `已生成 ${streaming.images.length} 张图片` : "");
  const activeStreamingAssistant = $derived.by(() => {
    const assistantId = streaming.assistantMessageId;
    if (!assistantId || !session) {
      return null;
    }
    return (
      session.thread_messages.find(
        (message) => message.id === assistantId && message.kind === "message" && message.role === "assistant"
      ) ?? null
    );
  });
  const liveThinkingMessageId = $derived.by(() => {
    if (!streaming.thinking || !session || !streaming.thinkingMessageId) {
      return null;
    }
    const message = session.thread_messages.find(
      (entry) => entry.id === streaming.thinkingMessageId && entry.kind === "thinking"
    );
    return message?.id ?? null;
  });
  const transientThinkingContent = $derived.by(() => {
    if (liveThinkingMessageId || !streaming.thinking) {
      return "";
    }
    return normalizedStreamingThinking;
  });
  const streamingSkeletonLayout = $derived.by(() => {
    const signalWeight =
      streaming.streamingContent.length +
      (streaming.thinking ? 23 : 0) +
      streaming.toolCalls.length * 19 +
      (streaming.reasoning ? 29 : 0) +
      streaming.images.length * 13;
    const variant = signalWeight % 4;

    if (variant === 0) {
      return ["78%", "61%", "70%"];
    }
    if (variant === 1) {
      return ["68%", "84%", "52%", "64%"];
    }
    if (variant === 2) {
      return ["86%", "58%", "74%"];
    }
    return ["72%", "49%", "81%", "57%"];
  });

  function scrollToBottom() {
    if (messageListRef) {
      requestAnimationFrame(() => {
        messageListRef!.scrollTo({ top: messageListRef!.scrollHeight, behavior: "smooth" });
      });
    }
  }

  // Auto-scroll on new streaming content
  $effect(() => {
    // Touch reactive deps
    void streaming.streamingContent;
    void streaming.thinking;
    void streaming.thinkingMessage;
    void session?.thread_messages.length;
    scrollToBottom();
  });

  $effect(() => {
    if (!composerSeed || composerSeed.id === lastComposerSeedId) {
      return;
    }

    draftMessage = draftMessage.trim()
      ? `${draftMessage.trim()}\n\n${composerSeed.content}`
      : composerSeed.content;
    lastComposerSeedId = composerSeed.id;
    autoResize();
    textareaRef?.focus();
  });

  $effect(() => {
    const threadId = session?.active_thread_id ?? null;
    if (threadId === lastReflectionThreadId) {
      return;
    }
    for (const timer of reflectionPollTimers.values()) {
      clearTimeout(timer);
    }
    reflectionPollTimers.clear();
    activeReflectionAssistantId = null;
    reflectionPanels = {};
    lastReflectionThreadId = threadId;
    lastReflectionSignalKey = null;
  });

  $effect(() => {
    const signal = streaming.reflectionSignal;
    const threadId = session?.active_thread_id ?? null;
    if (!signal || !threadId) {
      return;
    }

    const signalKey = `${threadId}:${signal.assistantMessageId}:${signal.sequence}:${signal.kind}`;
    if (signalKey === lastReflectionSignalKey) {
      return;
    }
    lastReflectionSignalKey = signalKey;

    if (signal.kind === "reflection_status" && signal.status) {
      seedReflectionLifecycleStatus(signal.assistantMessageId, signal.status);
    }

    if (activeReflectionAssistantId !== signal.assistantMessageId) {
      return;
    }

    const assistantMessage = session?.thread_messages.find(
      (message) =>
        message.id === signal.assistantMessageId &&
        message.kind === "message" &&
        message.role === "assistant"
    );
    if (!assistantMessage) {
      return;
    }

    void loadReflectionDetails(assistantMessage, true);
  });

  function toggleToolCallExpand(id: string) {
    const next = new Set(expandedToolCalls);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    expandedToolCalls = next;
  }

  function clearReflectionPoll(assistantId: string) {
    const timer = reflectionPollTimers.get(assistantId);
    if (timer) {
      clearTimeout(timer);
      reflectionPollTimers.delete(assistantId);
    }
  }

  function reflectionPanelForMessage(message: ThreadMessage) {
    return reflectionPanels[message.id] ?? null;
  }

  function observedReflectionTimelineForMessage(message: ThreadMessage) {
    return observedReflectionTimelines.get(message.id) ?? [];
  }

  function seedReflectionLifecycleStatus(assistantMessageId: string, status: ReflectionStatus) {
    const existing = reflectionPanels[assistantMessageId];
    const detail = existing?.detail
      ? { ...existing.detail, status }
      : {
          assistant_message_id: assistantMessageId,
          status,
          outcome: null,
          summary: null,
          detail: null,
          run_started_at: null,
          run_completed_at: null,
          tool_calls: [],
          messages: []
        };

    reflectionPanels = {
      ...reflectionPanels,
      [assistantMessageId]: {
        loading: existing?.loading ?? false,
        error: existing?.error ?? null,
        detail
      }
    };
  }

  function scheduleReflectionPoll(message: ThreadMessage) {
    const threadId = session?.active_thread_id ?? null;
    if (!threadId) {
      return;
    }
    clearReflectionPoll(message.id);
    reflectionPollTimers.set(
      message.id,
      setTimeout(() => {
        if (session?.active_thread_id !== threadId) {
          clearReflectionPoll(message.id);
          return;
        }
        void loadReflectionDetails(message, true);
      }, 1400)
    );
  }

  async function loadReflectionDetails(message: ThreadMessage, force = false) {
    if (!session) {
      return;
    }

    const existing = reflectionPanels[message.id];
    if (
      !force &&
      (existing?.loading ||
        (existing?.detail &&
          (existing.detail.summary !== null ||
            existing.detail.detail !== null ||
            existing.detail.tool_calls.length > 0 ||
            existing.detail.messages.length > 0 ||
            existing.detail.run_started_at !== null ||
            existing.detail.run_completed_at !== null ||
            existing.detail.status === "completed" ||
            existing.detail.status === "failed" ||
            existing.detail.status === "missing")))
    ) {
      return;
    }

    reflectionPanels = {
      ...reflectionPanels,
      [message.id]: {
        loading: true,
        error: null,
        detail: existing?.detail ?? null
      }
    };

    try {
      const detail = await apiClient.getReflectionDetails(session.active_thread_id, message.id);
      const optimisticStatus = existing?.detail?.status;
      const resolvedDetail =
        detail.status === "missing" &&
        (optimisticStatus === "queued" || optimisticStatus === "running")
          ? {
              ...detail,
              status: optimisticStatus
            }
          : detail;
      reflectionPanels = {
        ...reflectionPanels,
        [message.id]: {
          loading: false,
          error: null,
          detail: resolvedDetail
        }
      };

      if (
        (resolvedDetail.status === "queued" || resolvedDetail.status === "running") &&
        activeReflectionAssistantId === message.id
      ) {
        scheduleReflectionPoll(message);
      } else {
        clearReflectionPoll(message.id);
      }
    } catch (error) {
      clearReflectionPoll(message.id);
      reflectionPanels = {
        ...reflectionPanels,
        [message.id]: {
          loading: false,
          error: error instanceof Error ? error.message : "Failed to load reflection details.",
          detail: existing?.detail ?? null
        }
      };
    }
  }

  function closeReflectionModal() {
    if (activeReflectionAssistantId) {
      clearReflectionPoll(activeReflectionAssistantId);
    }
    activeReflectionAssistantId = null;
  }

  function toggleReflectionExpand(message: ThreadMessage) {
    if (activeReflectionAssistantId === message.id) {
      closeReflectionModal();
      return;
    }
    activeReflectionAssistantId = message.id;
    void loadReflectionDetails(message, true);
  }

  function toggleImagesExpand() {
    imagesExpanded = !imagesExpanded;
  }

  function toolCallDuration(tool: TimelineToolCall, createdAt?: string): string {
    const startedAt =
      "startedAt" in tool && typeof tool.startedAt === "string" ? tool.startedAt : createdAt;
    const completedAt =
      "completedAt" in tool && typeof tool.completedAt === "string" ? tool.completedAt : null;
    if (!startedAt) {
      return tool.status;
    }
    if (!completedAt) {
      if (tool.status !== "running") {
        return "";
      }
      const elapsed = Date.now() - new Date(startedAt).getTime();
      return `${(elapsed / 1000).toFixed(1)}s`;
    }
    const duration = new Date(completedAt).getTime() - new Date(startedAt).getTime();
    return `${(duration / 1000).toFixed(1)}s`;
  }

  function appendComposerFiles(files: FileList | File[]) {
    const next = Array.from(files)
      .filter((file) => file.size > 0)
      .map((file) => ({
        id: crypto.randomUUID(),
        file
      }));

    if (next.length === 0) {
      return;
    }

    composerAttachments = [...composerAttachments, ...next];
  }

  async function appendDroppedPaths(paths: string[]) {
    console.debug("[composer-dnd-tauri-read]", {
      pathCount: paths.length,
      paths
    });

    const results = await Promise.allSettled(
      paths.map(async (path) => {
        const dropped = await apiClient.readDroppedAttachmentFile(path);
        const dataBase64 =
          dropped.data_base64 ||
          (dropped as unknown as { dataBase64?: string }).dataBase64 ||
          "";
        const mimeType =
          dropped.mime_type ||
          (dropped as unknown as { mimeType?: string }).mimeType ||
          "application/octet-stream";

        if (!dataBase64) {
          throw new Error("Dropped attachment payload missing data_base64");
        }

        return new File([base64ToUint8Array(dataBase64)], dropped.filename, {
          type: mimeType
        });
      })
    );

    const files = results
      .filter((result): result is PromiseFulfilledResult<File> => result.status === "fulfilled")
      .map((result) => result.value);

    const failures = results.filter((result) => result.status === "rejected");
    if (failures.length > 0) {
      console.error("[composer-dnd-tauri-read-failed]", {
        failedCount: failures.length,
        errors: failures.map((result) => String(result.reason))
      });
    }

    appendComposerFiles(files);
  }

  function handleFileSelection(event: Event) {
    const input = event.currentTarget as HTMLInputElement | null;
    if (!input?.files?.length) {
      return;
    }
    appendComposerFiles(input.files);
    input.value = "";
  }

  function removeComposerAttachment(id: string) {
    composerAttachments = composerAttachments.filter((attachment) => attachment.id !== id);
  }

  function openFilePicker() {
    fileInputRef?.click();
  }

  function logComposerDragEvent(eventName: string, event: DragEvent) {
    const types = event.dataTransfer?.types ? Array.from(event.dataTransfer.types) : [];
    const fileNames = event.dataTransfer?.files
      ? Array.from(event.dataTransfer.files).map((file) => file.name)
      : [];

    console.debug("[composer-dnd]", {
      event: eventName,
      types,
      fileCount: event.dataTransfer?.files?.length ?? 0,
      fileNames
    });
  }

  function handleComposerDragEnter(event: DragEvent) {
    logComposerDragEvent("dragenter", event);
    if (!event.dataTransfer?.types?.includes("Files")) {
      return;
    }
    event.preventDefault();
    dragOverComposer = true;
  }

  function handleComposerDragOver(event: DragEvent) {
    logComposerDragEvent("dragover", event);
    if (!event.dataTransfer?.types?.includes("Files")) {
      return;
    }
    event.preventDefault();
    dragOverComposer = true;
  }

  function handleComposerDragLeave(event: DragEvent) {
    logComposerDragEvent("dragleave", event);
    const relatedTarget = event.relatedTarget as Node | null;
    if (relatedTarget && (event.currentTarget as HTMLElement | null)?.contains(relatedTarget)) {
      return;
    }
    dragOverComposer = false;
  }

  function handleComposerDrop(event: DragEvent) {
    logComposerDragEvent("drop", event);
    if (nativeFileDropBridgeActive) {
      event.preventDefault();
      dragOverComposer = false;
      console.debug("[composer-dnd] DOM drop ignored because native Tauri bridge is active");
      return;
    }
    if (!event.dataTransfer?.files?.length) {
      console.debug("[composer-dnd] drop ignored because dataTransfer.files is empty");
      return;
    }
    event.preventDefault();
    dragOverComposer = false;
    appendComposerFiles(event.dataTransfer.files);
  }

  function pointInsideComposer(x: number, y: number) {
    if (!inputBoxRef) {
      return false;
    }

    const scale = window.devicePixelRatio || 1;
    const candidates: Array<[number, number]> = [
      [x, y],
      [x / scale, y / scale]
    ];
    const rect = inputBoxRef.getBoundingClientRect();

    return candidates.some(([candidateX, candidateY]) => {
      const withinRect =
        candidateX >= rect.left &&
        candidateX <= rect.right &&
        candidateY >= rect.top &&
        candidateY <= rect.bottom;
      if (withinRect) {
        return true;
      }
      const hit = document.elementFromPoint(candidateX, candidateY);
      return !!hit && (hit === inputBoxRef || (inputBoxRef !== null && inputBoxRef.contains(hit)));
    });
  }

  function clearComposerAfterSend() {
    draftMessage = "";
    composerAttachments = [];

    if (textareaRef) {
      textareaRef.style.height = "auto";
    }
  }

  async function submitComposer(
    sender: (content: string, files: File[]) => Promise<boolean>
  ) {
    if (!canSubmit) return;
    const files = composerAttachments.map((attachment) => attachment.file);
    const sent = await sender(draftMessage, files);
    if (!sent) {
      return;
    }

    clearComposerAfterSend();
  }

  async function handleSubmit() {
    await submitComposer(onSendMessage);
  }

  async function handleSheerSubmit() {
    await submitComposer(onSheerSendMessage);
  }

  async function handleQueueSubmit() {
    await submitComposer(onQueueSendMessage);
  }

  async function handleInterruptClick() {
    sendControlHovered = false;
    await onInterruptSession();
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      if (!canSubmit) {
        return;
      }
      if (isBusySession) {
        void handleSheerSubmit();
        return;
      }
      void handleSubmit();
    }
  }

  function autoResize() {
    if (textareaRef) {
      textareaRef.style.height = "auto";
      textareaRef.style.height = `${Math.min(textareaRef.scrollHeight, 200)}px`;
    }
  }

  function handleModeToggle() {
    if (isYoloMode) {
      onChangeMessageMode("ask");
      showYoloRiskModal = false;
      return;
    }

    showYoloRiskModal = true;
  }

  function confirmYoloMode() {
    showYoloRiskModal = false;
    onChangeMessageMode("yolo");
  }

  function closeYoloRiskModal() {
    showYoloRiskModal = false;
  }

  function normalizeThinkingTranscript(value: string) {
    if (!value) {
      return "";
    }

    return value
      .replace(/(?:^|[\s\n])(?:正在处理|处理中)(?:\s*(?:\.{3}|…))?/giu, " ")
      .replace(/(?:^|[\s\n])processing(?:\s*(?:\.{3}|…))?/giu, " ")
      .replace(/(?:^|[\s\n])thinking(?:\s*\(step\s*\d+\))?(?:\s*(?:\.{3}|…))?/giu, " ")
      .replace(/(?:^|[\s\n])step\s*\d+(?:\s*(?:\.{3}|…))?/giu, " ")
      .replace(/[ \t]+\n/gu, "\n")
      .replace(/\n{3,}/gu, "\n\n")
      .replace(/[ \t]{2,}/gu, " ")
      .trim();
  }

  function buildCompactSummary(value: string, limit = 88) {
    const compact = value.replace(/\s+/gu, " ").trim();
    if (!compact) {
      return "";
    }
    if (compact.length <= limit) {
      return compact;
    }
    return `${compact.slice(0, limit).trim()}...`;
  }

  function buildTrailingSummary(value: string, limit = 92) {
    const compact = value.replace(/\s+/gu, " ").trim();
    if (!compact) {
      return "";
    }
    if (compact.length <= limit) {
      return compact;
    }
    return `...${compact.slice(-limit).trim()}`;
  }

  function toolCallSummary(tool: TimelineToolCall) {
    if (tool.error) {
      return buildCompactSummary(normalizeAuxiliaryText(tool.error), 96) || "执行失败";
    }
    if (tool.resultPreview) {
      return buildCompactSummary(normalizeAuxiliaryText(tool.resultPreview), 96);
    }
    if (tool.rationale) {
      return buildCompactSummary(normalizeAuxiliaryText(tool.rationale), 96);
    }
    if (tool.parameters) {
      return buildCompactSummary(normalizeAuxiliaryText(tool.parameters), 96);
    }
    if (tool.status === "running") {
      return "执行中...";
    }
    if (tool.status === "completed") {
      return "已完成";
    }
    return "已结束";
  }

  function stopTypingTimer() {
    if (typingTimer !== null) {
      clearTimeout(typingTimer);
      typingTimer = null;
    }
  }

  function stopThinkingTypingTimer() {
    if (thinkingTypingTimer !== null) {
      clearTimeout(thinkingTypingTimer);
      thinkingTypingTimer = null;
    }
  }

  function clearAuxiliarySummaryTimer(id: string) {
    const existing = auxiliarySummaryTimers.get(id);
    if (existing !== undefined) {
      clearTimeout(existing);
      auxiliarySummaryTimers.delete(id);
    }
  }

  function markAuxiliarySummarySettling(id: string) {
    clearAuxiliarySummaryTimer(id);
    const next = new Set(settlingAuxiliarySummaries);
    next.add(id);
    settlingAuxiliarySummaries = next;

    const timer = setTimeout(() => {
      const updated = new Set(settlingAuxiliarySummaries);
      updated.delete(id);
      settlingAuxiliarySummaries = updated;
      auxiliarySummaryTimers.delete(id);
    }, 260);

    auxiliarySummaryTimers.set(id, timer);
  }

  function isSettlingAuxiliarySummary(id: string) {
    return settlingAuxiliarySummaries.has(id);
  }

  function isLiveThinkingMessage(message: ThreadMessage) {
    return message.kind === "thinking" && message.id === liveThinkingMessageId;
  }

  function thinkingInlineSummary(message: ThreadMessage) {
    const normalized = isLiveThinkingMessage(message)
      ? displayedThinkingContent(message)
      : normalizeThinkingTranscript(message.content ?? "");
    if (isLiveThinkingMessage(message)) {
      return buildTrailingSummary(normalized, 92) || "思考中...";
    }
    return buildCompactSummary(normalized, 92);
  }

  function displayedThinkingContent(message: ThreadMessage) {
    if (message.id === animatedThinkingId) {
      return animatedThinkingText;
    }
    return normalizeThinkingTranscript(message.content ?? "");
  }

  function unwrapToolOutputEnvelope(value: string) {
    const trimmed = value.trim();
    if (!trimmed.startsWith("<tool_output") || !trimmed.endsWith("</tool_output>")) {
      return trimmed;
    }

    const start = trimmed.indexOf(">");
    if (start < 0) {
      return trimmed;
    }

    return trimmed.slice(start + 1, trimmed.length - "</tool_output>".length).trim();
  }

  function normalizeAuxiliaryText(value: string) {
    return unwrapToolOutputEnvelope(value).trim();
  }

  function renderAuxiliaryDetail(value: string) {
    const trimmed = normalizeAuxiliaryText(value);
    if (!trimmed) {
      return "";
    }

    const looksLikeJson =
      (trimmed.startsWith("{") && trimmed.endsWith("}")) ||
      (trimmed.startsWith("[") && trimmed.endsWith("]"));

    if (looksLikeJson) {
      try {
        const formatted = JSON.stringify(JSON.parse(trimmed), null, 2);
        return renderMarkdown(`\`\`\`json\n${formatted}\n\`\`\``);
      } catch {
        // Fall back to markdown rendering for non-strict JSON payloads.
      }
    }

    return renderMarkdown(trimmed);
  }

  function reflectionStatusLabel(status: ReflectionStatus | "loading" | "unloaded") {
    switch (status) {
      case "queued":
        return "排队中";
      case "running":
        return "进行中";
      case "completed":
        return "已完成";
      case "failed":
        return "失败";
      case "missing":
        return "暂无记录";
      case "loading":
        return "加载中";
      case "unloaded":
        return "查看";
      default:
        return "未知";
    }
  }

  function formatAttachmentSize(sizeBytes: number | null | undefined) {
    if (!sizeBytes || sizeBytes <= 0) {
      return "";
    }
    if (sizeBytes < 1024) {
      return `${sizeBytes} B`;
    }
    if (sizeBytes < 1024 * 1024) {
      return `${Math.max(1, Math.round(sizeBytes / 1024))} KB`;
    }
    return `${(sizeBytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  function attachmentDisplayName(attachment: ThreadMessageAttachment) {
    return attachment.filename || attachment.workspace_uri || "附件";
  }

  function base64ToUint8Array(value: string) {
    const binary = atob(value);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
    return bytes;
  }

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    void (async () => {
      unlisten = await listenForFileDrops(async (event) => {
        nativeFileDropBridgeActive = true;
        console.debug("[composer-dnd-tauri]", event);
        const previousNativeDragInside = nativeDragInsideComposer;

        if (event.type === "leave") {
          dragOverComposer = false;
          nativeDragInsideComposer = false;
          return;
        }

        const hitInside = event.position
          ? pointInsideComposer(event.position.x, event.position.y)
          : false;
        const inside =
          event.type === "drop"
            ? hitInside || previousNativeDragInside || dragOverComposer
            : hitInside;

        console.debug("[composer-dnd-tauri-hit]", {
          type: event.type,
          inside,
          hitInside,
          fallbackInside: previousNativeDragInside,
          position: event.position,
          paths: event.paths
        });

        if (event.type === "enter" || event.type === "over") {
          dragOverComposer = inside;
          nativeDragInsideComposer = inside;
          return;
        }

        if (event.type === "drop") {
          dragOverComposer = false;
          nativeDragInsideComposer = false;
          if (!inside || event.paths.length === 0) {
            console.debug("[composer-dnd-tauri-drop-ignored]", {
              inside,
              hitInside,
              fallbackInside: previousNativeDragInside,
              pathCount: event.paths.length
            });
            return;
          }
          await appendDroppedPaths(event.paths);
        }
      });

      if (disposed) {
        unlisten?.();
      }
    })();

    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  function reflectionStatusDescription(panel: ReflectionPanelState | null) {
    if (panel?.loading && !panel.detail) {
      return "当前轮次的 reflection 时间线。";
    }
    if (panel?.detail?.detail) {
      return panel.detail.detail;
    }
    if (panel?.detail?.status === "queued") {
      return "Reflection 已进入队列，正在等待执行槽位。";
    }
    if (panel?.detail?.status === "running") {
      return "Reflection 正在处理当前轮次。";
    }
    if (panel?.detail?.status === "completed") {
      switch (panel.detail.outcome) {
        case "boot_promoted":
          return "Reflection 已完成，并将记忆提升到了启动召回。";
        case "updated":
          return "Reflection 已完成，并更新了现有记忆。";
        case "created":
          return "Reflection 已完成，并写入了新记忆。";
        case "no_op":
          return "Reflection 已完成，并决定不写入新记忆。";
        default:
          return "当前轮次的 reflection 已完成。";
      }
    }
    if (panel?.detail?.status === "failed") {
      return "Reflection 在完成记忆更新前失败了。";
    }
    if (panel?.detail?.status === "missing") {
      return "当前轮次还没有找到 reflection 记录。";
    }
    if (panel?.detail?.summary) {
      return buildCompactSummary(panel.detail.summary, 160);
    }
    return "当前轮次的 reflection 时间线。";
  }

  function reflectionToolStatusLabel(status: TimelineToolCall["status"]) {
    switch (status) {
      case "running":
        return "进行中";
      case "completed":
        return "已完成";
      case "failed":
        return "失败";
      default:
        return status;
    }
  }

  function reflectionBadgeStatus(panel: ReflectionPanelState | null): ReflectionStatus | "loading" | "unloaded" {
    if (panel?.loading && !panel.detail) {
      return "loading";
    }
    return panel?.detail?.status ?? "unloaded";
  }

  function formatAuxiliaryTimestamp(value: string) {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) {
      return value;
    }
    return date.toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit"
    });
  }

  function displayedAssistantContent(message: ThreadMessage) {
    if (message.id === animatedAssistantId) {
      return animatedAssistantText;
    }
    return message.content ?? "";
  }

  $effect(() => {
    const targetId = activeStreamingAssistant?.id ?? null;
    const targetText = activeStreamingAssistant?.content ?? "";

    if (targetId !== animatedAssistantId) {
      stopTypingTimer();
      animatedAssistantId = targetId;
      animatedAssistantText = "";
    }

    if (!targetId) {
      animatedAssistantText = "";
      return;
    }

    if (animatedAssistantText.length > targetText.length) {
      animatedAssistantText = targetText;
      return;
    }

    if (animatedAssistantText.length === targetText.length) {
      stopTypingTimer();
      return;
    }

    stopTypingTimer();
    const remaining = targetText.length - animatedAssistantText.length;
    const stepSize = Math.max(1, Math.min(12, Math.ceil(remaining / 18)));
    typingTimer = setTimeout(() => {
      if (animatedAssistantId !== targetId) {
        return;
      }
      animatedAssistantText = targetText.slice(0, animatedAssistantText.length + stepSize);
    }, 14);

    return () => stopTypingTimer();
  });

  $effect(() => {
    const targetId = liveThinkingMessageId;
    const thinkingMessage = targetId && session
      ? session.thread_messages.find((message) => message.id === targetId && message.kind === "thinking") ?? null
      : null;
    const targetText = thinkingMessage ? normalizeThinkingTranscript(thinkingMessage.content ?? "") : "";

    if (targetId !== animatedThinkingId) {
      stopThinkingTypingTimer();
      animatedThinkingId = targetId;
      animatedThinkingText = "";
    }

    if (!targetId) {
      animatedThinkingText = "";
      return;
    }

    if (animatedThinkingText.length > targetText.length) {
      animatedThinkingText = targetText;
      return;
    }

    if (animatedThinkingText.length === targetText.length) {
      stopThinkingTypingTimer();
      return;
    }

    stopThinkingTypingTimer();
    const remaining = targetText.length - animatedThinkingText.length;
    const stepSize = Math.max(1, Math.min(10, Math.ceil(remaining / 20)));
    thinkingTypingTimer = setTimeout(() => {
      if (animatedThinkingId !== targetId) {
        return;
      }
      animatedThinkingText = targetText.slice(0, animatedThinkingText.length + stepSize);
    }, 18);

    return () => stopThinkingTypingTimer();
  });

  $effect(() => {
    const currentLiveThinkingId = liveThinkingMessageId;
    if (lastLiveThinkingId && lastLiveThinkingId !== currentLiveThinkingId) {
      markAuxiliarySummarySettling(lastLiveThinkingId);
    }
    lastLiveThinkingId = currentLiveThinkingId;
  });

  onDestroy(() => {
    stopTypingTimer();
    stopThinkingTypingTimer();
    for (const timer of auxiliarySummaryTimers.values()) {
      clearTimeout(timer);
    }
    auxiliarySummaryTimers.clear();
    for (const timer of reflectionPollTimers.values()) {
      clearTimeout(timer);
    }
    reflectionPollTimers.clear();
  });
</script>

<div class="chat-area" class:empty-mode={showEmptyLayout}>
  <!-- Messages Area -->
  {#if loading}
    <div class="loading-state">
      <div class="loading-spinner"></div>
      <p>加载中...</p>
    </div>
  {:else if showEmptyLayout}
    <div
      class="empty-chat"
      aria-hidden="true"
      in:fade={{ duration: 180 }}
      out:fade={{ duration: 160 }}
    ></div>
  {:else}
    <div
      class="message-list"
      bind:this={messageListRef}
      in:fade={{ duration: 220 }}
      out:fade={{ duration: 140 }}
    >
      {#each displayEntries as entry, idx (entry.kind === "message" ? entry.message.id : entry.id)}
        <div
          class="message {(entry.kind === 'auxiliary_group') ? 'assistant' : (entry.message.role ?? entry.message.kind)} fade-in"
          style="animation-delay: {Math.min(idx * 30, 300)}ms"
        >
                    {#if entry.kind === "auxiliary_group"}
            <div class="assistant-text">
              <div class="tool-calls-container inline-tool-call">
                <div class="tool-call-card tool-group-card aux-group-card">
                  <div class="aux-scroll-area">
                  {#each entry.messages as message, groupIndex}
                    <div class="tool-group-row">
                      <button class="tool-call-header tool-group-row-header" onclick={() => toggleToolCallExpand(message.id)}>
                        <div class="tool-call-left">
                          {#if message.kind === "thinking"}
                            <div class="tool-icon thinking">
                              <Brain size={14} strokeWidth={2} />
                            </div>
                            <div class="tool-call-copy tool-row-copy">
                              <div class="tool-row-inline">
                                <span class="tool-name">思考</span>
                                <span
                                  class="tool-inline-summary"
                                  class:live-tool-summary={isLiveThinkingMessage(message)}
                                  class:summary-settle={isSettlingAuxiliarySummary(message.id)}
                                >
                                  {thinkingInlineSummary(message)}
                                </span>
                              </div>
                            </div>
                          {:else}
                            <div class="tool-icon tool-kind-icon">
                              <Wrench size={14} strokeWidth={2} />
                            </div>
                            <div class="tool-call-copy tool-row-copy">
                              <div class="tool-row-inline">
                                <span class="tool-name">{message.tool_call?.name}</span>
                                {#if message.tool_call?.status === "running"}
                                  <span class="tool-inline-loader" aria-label="工具调用进行中">
                                    <Loader size={13} strokeWidth={2} />
                                  </span>
                                {/if}
                                <span
                                  class="tool-inline-summary"
                                  class:tool-inline-error={message.tool_call?.status === "failed"}
                                >
                                  {toolCallSummary(message.tool_call as TimelineToolCall)}
                                </span>
                              </div>
                            </div>
                          {/if}
                        </div>
                        <div class="tool-call-right">
                          {#if message.kind === "tool_call" && message.tool_call}
                            <span class="tool-duration">{toolCallDuration(message.tool_call as TimelineToolCall, message.created_at)}</span>
                          {/if}
                          <ChevronRight size={14} strokeWidth={2} class="expand-icon {expandedToolCalls.has(message.id) ? 'expanded' : ''}" />
                        </div>
                      </button>
                      {#if expandedToolCalls.has(message.id)}
                        <div class="tool-call-body slide-down">
                          {#if message.kind === "thinking"}
                            <div class="thinking-segment">
                              <div class="tool-detail-content auxiliary-detail markdown-body detail-enter">
                                {@html renderAuxiliaryDetail(displayedThinkingContent(message))}
                              </div>
                            </div>
                          {:else}
                            {#if message.tool_call?.rationale}
                              <div class="tool-detail">
                                <span class="tool-detail-label">原因</span>
                                <div class="tool-detail-content auxiliary-detail markdown-body detail-enter">
                                  {@html renderAuxiliaryDetail(message.tool_call.rationale)}
                                </div>
                              </div>
                            {/if}
                            {#if message.tool_call?.parameters}
                              <div class="tool-detail">
                                <span class="tool-detail-label">参数</span>
                                <div class="tool-detail-content auxiliary-detail markdown-body detail-enter">
                                  {@html renderAuxiliaryDetail(message.tool_call.parameters)}
                                </div>
                              </div>
                            {/if}
                            {#if message.tool_call?.resultPreview}
                              <div class="tool-detail">
                                <span class="tool-detail-label">结果</span>
                                <div class="tool-detail-content auxiliary-detail markdown-body detail-enter">
                                  {@html renderAuxiliaryDetail(message.tool_call.resultPreview)}
                                </div>
                              </div>
                            {/if}
                            {#if message.tool_call?.error}
                              <div class="tool-detail error-detail">
                                <span class="tool-detail-label">错误</span>
                                <div class="tool-detail-content auxiliary-detail markdown-body detail-enter">
                                  {@html renderAuxiliaryDetail(message.tool_call.error)}
                                </div>
                              </div>
                            {/if}
                          {/if}
                        </div>
                      {/if}
                    </div>
                    {#if groupIndex < entry.messages.length - 1}
                      <div class="tool-group-divider"></div>
                    {/if}
                  {/each}
                  </div>
                </div>
              </div>
            </div>
          {:else if entry.message.role === "user"}
            <div class="user-message-stack">
              {#if entry.message.content}
                <div class="user-bubble">
                  {entry.message.content}
                </div>
              {/if}
              {#if entry.message.attachments.length > 0}
                <div class="user-attachments">
                  {#each entry.message.attachments as attachment}
                    <div class="attachment-chip message-attachment-chip">
                      <div class="attachment-chip-icon">
                        {#if attachment.kind === "image"}
                          <Image size={14} strokeWidth={2} />
                        {:else if attachment.kind === "audio"}
                          <Music4 size={14} strokeWidth={2} />
                        {:else}
                          <FileText size={14} strokeWidth={2} />
                        {/if}
                      </div>
                      <div class="attachment-chip-copy">
                        <span class="attachment-chip-name">{attachmentDisplayName(attachment)}</span>
                        <span class="attachment-chip-meta">
                          {attachment.kind}
                          {#if attachment.size_bytes}
                            <span>· {formatAttachmentSize(attachment.size_bytes)}</span>
                          {/if}
                        </span>
                      </div>
                    </div>
                  {/each}
                </div>
              {/if}
            </div>
          {:else}
            <div class="assistant-text">
              <div class="assistant-content markdown-body">
                {@html renderMarkdown(displayedAssistantContent(entry.message))}
              </div>
              {#if entry.message.turn_cost}
                {@const reflectionPanel = reflectionPanelForMessage(entry.message)}
                {@const reflectionTimeline = observedReflectionTimelineForMessage(entry.message)}
                {@const reflectionBadge = reflectionBadgeStatus(reflectionPanel)}
                <div class="turn-meta-stack">
                  <div class="turn-cost-bar inline-turn-cost fade-in">
                    <div class="turn-cost-copy">
                      <Zap size={12} strokeWidth={2} />
                      <span>{entry.message.turn_cost.input_tokens.toLocaleString()} 输入</span>
                      <span class="cost-sep">·</span>
                      <span>{entry.message.turn_cost.output_tokens.toLocaleString()} 输出</span>
                      {#if entry.message.turn_cost.cost_usd !== "$0.0000"}
                        <span class="cost-sep">·</span>
                        <span>{entry.message.turn_cost.cost_usd}</span>
                      {/if}
                    </div>
                    <button
                      class="reflection-trigger"
                      class:is-open={activeReflectionAssistantId === entry.message.id}
                      type="button"
                      onclick={() => toggleReflectionExpand(entry.message)}
                      aria-label="打开 reflection 详情"
                      title="Reflection 详情"
                    >
                      <Brain size={12} strokeWidth={1.9} />
                      <span class={`reflection-trigger-dot status-${reflectionBadge}`}></span>
                    </button>
                  </div>
                </div>
              {/if}
            </div>
          {/if}
        </div>
      {/each}

      {#if transientThinkingContent}
        <div class="message assistant fade-in">
          <div class="assistant-text">
            <div class="tool-calls-container inline-tool-call">
              <div class="tool-call-card tool-group-card aux-group-card">
                <div class="aux-scroll-area">
                  <div class="tool-group-row">
                    <button class="tool-call-header tool-group-row-header" disabled>
                      <div class="tool-call-left">
                        <div class="tool-icon thinking">
                          <Brain size={14} strokeWidth={2} />
                        </div>
                        <div class="tool-call-copy tool-row-copy">
                          <div class="tool-row-inline">
                            <span class="tool-name">思考</span>
                            <span class="tool-inline-summary live-tool-summary">
                              {buildTrailingSummary(transientThinkingContent, 92) || "思考中..."}
                            </span>
                          </div>
                        </div>
                      </div>
                    </button>
                    <div class="tool-call-body">
                      <div class="thinking-segment">
                        <div class="tool-detail-content auxiliary-detail markdown-body detail-enter">
                          {@html renderAuxiliaryDetail(transientThinkingContent)}
                        </div>
                      </div>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      {/if}

      {#if streaming.isStreaming && !hasLiveStreamingSignal}
        <div class="message assistant pending-request-message fade-in">
          <div class="assistant-text">
            <div class="pending-inline-skeleton" aria-hidden="true">
              {#each streamingSkeletonLayout as width, skeletonIndex}
                <span
                  class="pending-line"
                  style={`width: ${width}; animation-delay: ${skeletonIndex * 0.12}s;`}
                ></span>
              {/each}
            </div>
          </div>
        </div>
      {/if}

      {#if hasStreamingContent}
        <div class="message assistant streaming-message fade-in">
          <div class="assistant-text">
            <!-- Generated images -->
            {#if streaming.images.length > 0}
              <div class="tool-calls-container inline-tool-call">
                <div class="tool-call-card image-card" class:expanded={imagesExpanded}>
                  <button class="tool-call-header" onclick={toggleImagesExpand}>
                    <div class="tool-call-left">
                      <div class="tool-icon thinking">
                        <Image size={14} strokeWidth={2} />
                      </div>
                      <div class="tool-call-copy">
                        <div class="tool-call-title-row">
                          <span class="tool-name">生成图片</span>
                        </div>
                        <div class="tool-summary">{imageSummary}</div>
                      </div>
                    </div>
                    <div class="tool-call-right">
                      <ChevronRight size={14} strokeWidth={2} class="expand-icon" />
                    </div>
                  </button>
                  {#if imagesExpanded}
                    <div class="tool-call-body slide-down auxiliary-card-body">
                      <div class="image-gallery">
                        {#each streaming.images as img}
                          <div class="generated-image">
                            <img src={img.dataUrl} alt={img.path ?? "Generated image"} />
                            {#if img.path}
                              <span class="image-path">{img.path}</span>
                            {/if}
                          </div>
                        {/each}
                      </div>
                    </div>
                  {/if}
                </div>
              </div>
            {/if}
          </div>
        </div>
      {/if}

      <!-- Suggestions -->
      {#if streaming.suggestions.length > 0 && !streaming.isStreaming}
        <div class="suggestions-row fade-in">
          {#each streaming.suggestions as suggestion}
            <button class="suggestion-chip" onclick={() => onSuggestionClick?.(suggestion)}>
              {suggestion}
            </button>
          {/each}
        </div>
      {/if}

    </div>
  {/if}

  {#if task?.pending_approval}
    <TaskApprovalCard
      {task}
      modal={true}
      bind:rejectReason
      onApprove={() => task && onApproveTask(task)}
      onApproveAlways={() => task && onApproveTaskAlways(task)}
      onReject={() => task && onRejectTask(task, rejectReason)}
    />
  {/if}

  <!-- Input Area -->
  <div class="input-container" class:empty-mode={showEmptyLayout}>
    {#if showEmptyLayout}
      <div
        class="empty-composer-copy"
        in:fly={{ y: 18, duration: 260 }}
        out:fade={{ duration: 160 }}
      >
        <h2 class="empty-composer-title">今天想要做什么？</h2>
      </div>
    {/if}

    <div
      class="input-box"
      bind:this={inputBoxRef}
      class:empty-mode={showEmptyLayout}
      class:drag-active={dragOverComposer}
      role="group"
      aria-label="消息输入框"
      ondragenter={handleComposerDragEnter}
      ondragover={handleComposerDragOver}
      ondragleave={handleComposerDragLeave}
      ondrop={handleComposerDrop}
    >
      {#if noBackend}
        <div class="input-no-backend-hint">
          请先在设置中配置模型
        </div>
      {:else}
        <input
          bind:this={fileInputRef}
          type="file"
          class="composer-file-input"
          multiple
          onchange={handleFileSelection}
        />
        <textarea
          bind:this={textareaRef}
          bind:value={draftMessage}
          onkeydown={handleKeydown}
          oninput={autoResize}
          class="input-textarea"
          placeholder="发送消息..."
          rows="1"
        ></textarea>

        {#if composerAttachments.length > 0}
          <div class="composer-attachments">
            {#each composerAttachments as attachment}
              <button
                class="attachment-chip composer-attachment-chip"
                type="button"
                onclick={() => removeComposerAttachment(attachment.id)}
                aria-label={`移除附件 ${attachment.file.name}`}
              >
                <div class="attachment-chip-icon">
                  {#if attachment.file.type.startsWith("image/")}
                    <Image size={14} strokeWidth={2} />
                  {:else if attachment.file.type.startsWith("audio/")}
                    <Music4 size={14} strokeWidth={2} />
                  {:else}
                    <FileText size={14} strokeWidth={2} />
                  {/if}
                </div>
                <div class="attachment-chip-copy">
                  <span class="attachment-chip-name">{attachment.file.name}</span>
                  <span class="attachment-chip-meta">{formatAttachmentSize(attachment.file.size)}</span>
                </div>
                <span class="attachment-chip-remove">
                  <X size={13} strokeWidth={2.2} />
                </span>
              </button>
            {/each}
          </div>
        {/if}

        <div class="input-toolbar">
          <div class="input-actions-left">
            <button class="input-chip" type="button" aria-label="选择文件" onclick={openFilePicker}>
              <Paperclip size={15} strokeWidth={2} />
              <span>选择文件</span>
            </button>
            <button
              class={`input-chip mode-chip ${isYoloMode ? "active" : ""}`}
              type="button"
              aria-pressed={isYoloMode}
              onclick={handleModeToggle}
            >
              <Shield size={15} strokeWidth={2} />
              <span>{isYoloMode ? "权限 · 全自动" : "权限 · 需确认"}</span>
            </button>
          </div>

          <div class="input-actions-right">
            {#if contextStats}
              <button
                class="context-ring-btn"
                type="button"
                onclick={() => showContextStatsModal = true}
                title="上下文统计"
                aria-label="上下文统计"
              >
                <svg width="18" height="18" viewBox="0 0 18 18" xmlns="http://www.w3.org/2000/svg">
                  <circle cx="9" cy="9" r="7" stroke="var(--border-default)" stroke-width="2" fill="none" />
                  <circle cx="9" cy="9" r="7" stroke="var(--accent-primary)" stroke-width="2" fill="none" stroke-linecap="round" stroke-dasharray="43.98" stroke-dashoffset={43.98 * (1 - contextUsagePercent / 100)} transform="rotate(-90 9 9)" />
                </svg>
              </button>
            {/if}
            {#if isBusySession && canSubmit}
              <button
                class="send-btn send-btn-stop active"
                type="button"
                onclick={handleInterruptClick}
                title="停止当前运行"
                aria-label="停止当前运行"
              >
                <Square size={15} strokeWidth={2.2} />
              </button>
              <button
                class="send-btn send-btn-secondary active"
                type="button"
                onclick={handleSheerSubmit}
                title="sheer 发送，插到队列最前"
                aria-label="sheer 发送"
              >
                <ChevronRight size={16} strokeWidth={2.2} />
              </button>
              <button
                class="send-btn send-btn-secondary active"
                type="button"
                onclick={handleQueueSubmit}
                title="queue 发送，追加到队列末尾"
                aria-label="queue 发送"
              >
                <CornerDownRight size={16} strokeWidth={2.2} />
              </button>
            {:else if isBusySession}
              <button
                class={`send-btn ${sendControlHovered ? "active send-btn-stop" : "send-btn-busy"}`}
                type="button"
                onmouseenter={() => sendControlHovered = true}
                onmouseleave={() => sendControlHovered = false}
                onclick={handleInterruptClick}
                title={sendControlHovered ? "停止当前运行" : "当前正在运行"}
                aria-label={sendControlHovered ? "停止当前运行" : "当前正在运行"}
              >
                {#if sendControlHovered}
                  <Square size={15} strokeWidth={2.2} />
                {:else}
                  <Loader size={16} strokeWidth={2.2} class="spin" />
                {/if}
              </button>
            {:else}
              <button
                class="send-btn {canSubmit ? 'active' : ''}"
                type="button"
                onclick={handleSubmit}
                disabled={!canSubmit}
                title={canSubmit ? "发送" : "请输入消息"}
                aria-label={canSubmit ? "发送消息" : "请输入消息"}
              >
                <ArrowUp size={16} strokeWidth={2.2} />
              </button>
            {/if}
          </div>
        </div>
      {/if}
    </div>
  </div>

  {#if showYoloRiskModal}
    <div
      class="mode-modal-backdrop"
      role="presentation"
      tabindex="-1"
      transition:fade={{ duration: 140 }}
      onclick={closeYoloRiskModal}
      onkeydown={(event) => {
        if (event.key === "Escape" || event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          closeYoloRiskModal();
        }
      }}
    >
      <div
        class="mode-modal"
        role="dialog"
        aria-modal="true"
        aria-label="全自动模式风险提示"
        tabindex="-1"
        transition:fly={{ y: 18, duration: 180 }}
        onclick={(event) => event.stopPropagation()}
        onkeydown={(event) => {
          event.stopPropagation();
          if (event.key === "Escape") {
            event.preventDefault();
            closeYoloRiskModal();
          }
        }}
      >
        <div class="mode-modal-head">
          <div>
            <p class="eyebrow">Yolo Mode</p>
            <h3>切换到全自动模式</h3>
          </div>
          <button
            class="mode-modal-close"
            type="button"
            aria-label="关闭风险提示"
            onclick={closeYoloRiskModal}
          >
            <X size={16} strokeWidth={2.2} />
          </button>
        </div>

        <div class="mode-modal-body">
          <p>
            全自动模式会自动批准当前线程里所有原本需要 Ask 确认的操作，包括文件写入、命令执行、网络请求以及后续新的高风险步骤。
          </p>
          <p>
            只有在你信任当前上下文、工作区和模型输出时才应启用。如果当前任务正卡在审批点，切换后会立即自动继续执行。
          </p>
        </div>

        <div class="mode-modal-actions">
          <button class="button button-ghost mode-action-btn" type="button" onclick={closeYoloRiskModal}>
            继续 Ask
          </button>
          <button class="button button-primary mode-action-btn" type="button" onclick={confirmYoloMode}>
            继续开启
          </button>
        </div>
      </div>
    </div>
  {/if}

  {#if showContextStatsModal && contextStats}
    <div
      class="mode-modal-backdrop"
      role="presentation"
      tabindex="-1"
      transition:fade={{ duration: 140 }}
      onclick={() => showContextStatsModal = false}
      onkeydown={(event) => {
        if (event.key === "Escape" || event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          showContextStatsModal = false;
        }
      }}
    >
      <div
        class="mode-modal"
        role="dialog"
        aria-modal="true"
        aria-label="上下文统计"
        tabindex="-1"
        transition:fly={{ y: 18, duration: 180 }}
        onclick={(event) => event.stopPropagation()}
        onkeydown={(event) => {
          event.stopPropagation();
          if (event.key === "Escape") {
            event.preventDefault();
            showContextStatsModal = false;
          }
        }}
      >
        <div class="mode-modal-head">
          <div>
            <p class="eyebrow">Context</p>
            <h3>上下文统计</h3>
          </div>
          <button
            class="mode-modal-close"
            type="button"
            aria-label="关闭"
            onclick={() => showContextStatsModal = false}
          >
            <X size={16} strokeWidth={2.2} />
          </button>
        </div>

        <div class="mode-modal-body">
          <div class="context-stats-summary">
            <span>模型上下文窗口: <strong>{session?.model_context_length?.toLocaleString() ?? 'N/A'} tokens</strong></span>
          </div>
          <div class="context-stats-grid">
            <div class="context-stat-row">
              <span class="context-stat-label">System Prompt</span>
              <span class="context-stat-value">{contextStats.system_prompt_tokens.toLocaleString()} tokens</span>
            </div>
            <div class="context-stat-row">
              <span class="context-stat-label">MCP Prompts</span>
              <span class="context-stat-value">{contextStats.mcp_prompts_tokens.toLocaleString()} tokens</span>
            </div>
            <div class="context-stat-row">
              <span class="context-stat-label">Skills</span>
              <span class="context-stat-value">{contextStats.skills_tokens.toLocaleString()} tokens</span>
            </div>
            <div class="context-stat-row">
              <span class="context-stat-label">Messages</span>
              <span class="context-stat-value">{contextStats.messages_tokens.toLocaleString()} tokens</span>
            </div>
            <div class="context-stat-row">
              <span class="context-stat-label">Tool Use</span>
              <span class="context-stat-value">{contextStats.tool_use_tokens.toLocaleString()} tokens</span>
            </div>
            <div class="context-stat-row">
              <span class="context-stat-label">Compact Buffer</span>
              <span class="context-stat-value">{contextStats.compact_buffer_tokens.toLocaleString()} tokens</span>
            </div>
            <div class="context-stat-row context-stat-free">
              <span class="context-stat-label">Free Space</span>
              <span class="context-stat-value" class:warning={contextStats.free_tokens < 0}>{contextStats.free_tokens.toLocaleString()} tokens</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  {/if}

  {#if activeReflectionMessage}
    {@const activeReflectionPanel = reflectionPanelForMessage(activeReflectionMessage)}
    {@const activeReflectionTimeline = observedReflectionTimelineForMessage(activeReflectionMessage)}
    {@const activeReflectionBadge = reflectionBadgeStatus(activeReflectionPanel)}
    <div
      class="reflection-modal-backdrop fade-in"
      role="presentation"
      tabindex="-1"
      onclick={closeReflectionModal}
      onkeydown={(event) => {
        if (event.key === "Escape" || event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          closeReflectionModal();
        }
      }}
    >
      <div
        class="reflection-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Reflection 详情"
        tabindex="-1"
        onclick={(event) => event.stopPropagation()}
        onkeydown={(event) => {
          event.stopPropagation();
          if (event.key === "Escape") {
            event.preventDefault();
            closeReflectionModal();
          }
        }}
      >
        <div class="reflection-modal-head">
          <div class="reflection-modal-title-wrap">
            <div class="reflection-panel-title-row">
              <span class="reflection-panel-title">Reflection 详情</span>
              <span class={`reflection-pill large status-${activeReflectionBadge}`}>
                {reflectionStatusLabel(activeReflectionBadge)}
              </span>
            </div>
            <p class="reflection-panel-subtitle">
              {reflectionStatusDescription(activeReflectionPanel)}
            </p>
          </div>
          <button
            class="reflection-modal-close"
            type="button"
            onclick={closeReflectionModal}
            aria-label="关闭 reflection 详情"
          >
            <X size={16} strokeWidth={2} />
          </button>
        </div>

        <div class="reflection-modal-body">
          {#if activeReflectionPanel?.loading && !activeReflectionPanel.detail}
            <div class="reflection-loading">
              <Loader size={15} class="spin" strokeWidth={2} />
              <span>正在同步 reflection 状态...</span>
            </div>
          {/if}

          <div class="reflection-section">
            <div class="reflection-section-head">
              <span>时间线</span>
              <span>{activeReflectionTimeline.length}</span>
            </div>
            {#if activeReflectionTimeline.length > 0}
              <div class="reflection-list">
                {#each activeReflectionTimeline as timelineEntry}
                  <div class="reflection-list-item">
                    <div class="reflection-item-header">
                      <div class="reflection-item-title">
                        {#if timelineEntry.kind === "thinking"}
                          <Brain size={13} strokeWidth={2} />
                          <strong>思考</strong>
                        {:else if timelineEntry.kind === "tool"}
                          <Wrench size={13} strokeWidth={2} />
                          <strong>{timelineEntry.toolCall.name}</strong>
                          <span class={`reflection-inline-status tool-${timelineEntry.toolCall.status}`}>
                            {reflectionToolStatusLabel(timelineEntry.toolCall.status)}
                          </span>
                        {:else}
                          <Sparkles size={13} strokeWidth={2} />
                          <strong>Reflection 消息</strong>
                        {/if}
                      </div>
                      <span class="reflection-item-time">
                        {formatAuxiliaryTimestamp(timelineEntry.createdAt)}
                      </span>
                    </div>

                    {#if timelineEntry.kind === "thinking"}
                      <div class="reflection-detail-body markdown-body">
                        {@html renderMarkdown(timelineEntry.content)}
                      </div>
                    {:else if timelineEntry.kind === "tool"}
                      {#if timelineEntry.toolCall.parameters}
                        <div class="reflection-detail-block">
                          <span class="reflection-detail-label">参数</span>
                          <div class="reflection-detail-body markdown-body">
                            {@html renderAuxiliaryDetail(timelineEntry.toolCall.parameters)}
                          </div>
                        </div>
                      {/if}
                      {#if timelineEntry.toolCall.resultPreview}
                        <div class="reflection-detail-block">
                          <span class="reflection-detail-label">结果</span>
                          <div class="reflection-detail-body markdown-body">
                            {@html renderAuxiliaryDetail(timelineEntry.toolCall.resultPreview)}
                          </div>
                        </div>
                      {/if}
                      {#if timelineEntry.toolCall.error}
                        <div class="reflection-detail-block error">
                          <span class="reflection-detail-label">错误</span>
                          <div class="reflection-detail-body markdown-body">
                            {@html renderAuxiliaryDetail(timelineEntry.toolCall.error)}
                          </div>
                        </div>
                      {/if}
                    {:else}
                      <div class="reflection-detail-body markdown-body">
                        {@html renderMarkdown(timelineEntry.content)}
                      </div>
                    {/if}
                  </div>
                {/each}
              </div>
            {/if}

            {#if activeReflectionPanel?.detail?.status === "queued" || activeReflectionPanel?.detail?.status === "running"}
              <div class="reflection-live-skeleton" aria-hidden="true">
                <div class="reflection-skeleton-line short"></div>
                <div class="reflection-skeleton-line full"></div>
                <div class="reflection-skeleton-line medium"></div>
              </div>
            {:else if activeReflectionTimeline.length === 0}
              <p class="reflection-empty">当前轮次暂时还没有记录到 reflection 活动。</p>
            {/if}
          </div>
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .chat-area {
    position: relative;
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--bg-primary);
    height: 100%;
  }

  .chat-area.empty-mode {
    justify-content: center;
  }

  /* Loading */
  .loading-state {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: var(--text-muted);
  }

  .loading-spinner {
    width: 24px;
    height: 24px;
    border: 2px solid var(--border-default);
    border-top-color: var(--text-secondary);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* Empty state */
  .empty-chat {
    flex: 1;
    min-height: 0;
    pointer-events: none;
  }

  .chat-area.empty-mode .empty-chat {
    display: none;
  }

  /* Messages */
  .message-list {
    flex: 1;
    overflow-y: auto;
    padding: 24px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    scroll-behavior: smooth;
  }

  .message {
    display: flex;
    max-width: 100%;
  }

  .message.user {
    justify-content: flex-end;
  }

  .message.assistant {
    justify-content: center;
    width: 100%;
  }

  .fade-in {
    animation: fadeSlideIn 0.3s ease both;
  }

  @keyframes fadeSlideIn {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .user-bubble {
    max-width: 70%;
    padding: 12px 18px;
    border-radius: 20px 20px 4px 20px;
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-size: 15px;
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .user-message-stack {
    width: min(70%, 520px);
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 8px;
  }

  .user-attachments {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .attachment-chip {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    border-radius: 16px;
    border: 1px solid var(--border-default);
    background: color-mix(in srgb, var(--bg-surface) 84%, white 16%);
    color: var(--text-primary);
    text-align: left;
  }

  .attachment-chip-icon {
    width: 30px;
    height: 30px;
    border-radius: 999px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: color-mix(in srgb, var(--accent-primary) 14%, transparent);
    color: var(--accent-primary);
    flex-shrink: 0;
  }

  .attachment-chip-copy {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
  }

  .attachment-chip-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
    font-weight: 600;
  }

  .attachment-chip-meta {
    font-size: 12px;
    color: var(--text-secondary);
  }

  .message-attachment-chip {
    padding: 10px 12px;
    box-shadow: var(--shadow-card);
  }

  .assistant-text {
    width: min(88%, 980px);
    max-width: none;
    padding: 1px 0;
    color: var(--text-primary);
    font-size: 15px;
    line-height: 1.38;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .assistant-content {
    width: 100%;
    white-space: normal;
    word-break: break-word;
  }

  .assistant-content :global(p),
  .assistant-content :global(li) {
    line-height: 1.64 !important;
  }

  .assistant-content :global(p) {
    margin: 0.28em 0 !important;
  }

  .assistant-content :global(ul),
  .assistant-content :global(ol) {
    margin: 0.38em 0 !important;
  }

  .assistant-content :global(blockquote),
  .assistant-content :global(pre),
  .assistant-content :global(table) {
    margin: 0.68em 0;
  }

  .thinking-text {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-size: 13px;
    line-height: 1.6;
    color: var(--text-secondary);
  }

  /* Reasoning block */
  .reasoning-block {
    padding: 10px 14px;
    border-radius: 12px;
    background: var(--bg-elevated);
    border-left: 3px solid var(--accent-gold);
    animation: fadeSlideIn 0.3s ease both;
  }

  .reasoning-header {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    font-weight: 600;
    color: var(--accent-gold);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin-bottom: 6px;
  }

  .reasoning-text {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
    color: var(--text-secondary);
  }

  .reasoning-decisions {
    margin-top: 8px;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .decision-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 10px;
    border-radius: 8px;
    background: var(--bg-hover);
    font-size: 12px;
    color: var(--text-secondary);
  }

  .decision-tool {
    font-weight: 600;
    color: var(--text-primary);
  }

  .decision-rationale {
    color: var(--text-tertiary);
  }

  /* Tool calls */
  .tool-calls-container {
    display: flex;
    flex-direction: column;
    gap: 6px;
    animation: fadeSlideIn 0.3s ease both;
  }

  .inline-tool-call {
    margin-top: 0;
  }

  .tool-call-card {
    width: 100%;
    border-radius: 12px;
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
    overflow: hidden;
    transition: border-color 0.2s ease, box-shadow 0.2s ease, background-color 0.2s ease;
  }

  .tool-call-card.running {
    border-color: var(--accent-gold);
    box-shadow: 0 0 0 1px rgba(201, 150, 58, 0.15);
  }

  .tool-call-card.completed {
    border-color: rgba(74, 222, 128, 0.3);
  }

  .tool-call-card.failed {
    border-color: rgba(239, 68, 68, 0.3);
  }

  .tool-call-card.thinking-card {
    border-color: color-mix(in srgb, var(--accent-primary) 30%, var(--border-default));
    background: color-mix(in srgb, var(--bg-surface) 90%, var(--accent-primary) 10%);
  }

  .tool-call-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 10px 14px;
    background: transparent;
    border: none;
    cursor: pointer;
    text-align: left;
    color: inherit;
    font-family: inherit;
    transition: background 0.12s ease;
  }

  .tool-call-header:hover {
    background: var(--bg-hover);
  }

  .tool-call-left {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    min-width: 0;
    flex: 1;
  }

  .tool-call-copy {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
  }

  .tool-call-title-row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .tool-spinner {
    color: var(--accent-gold);
    display: flex;
    animation: spin 1.2s linear infinite;
  }

  .tool-icon {
    display: flex;
    flex-shrink: 0;
  }

  .tool-icon.success {
    color: var(--accent-green);
    display: flex;
  }

  .tool-icon.error {
    color: var(--accent-danger-text);
    display: flex;
  }

  .tool-icon.thinking {
    color: var(--accent-primary);
    display: flex;
  }

  .tool-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    font-family: "SF Mono", "Fira Code", "Cascadia Code", monospace;
  }

  .tool-duration {
    font-size: 12px;
    color: var(--text-muted);
    flex-shrink: 0;
  }

  .tool-summary {
    font-size: 12px;
    line-height: 1.45;
    color: var(--text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tool-call-right {
    color: var(--text-muted);
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .expand-icon {
    transition: transform 0.2s ease;
  }

  .expand-icon.expanded {
    transform: rotate(90deg);
  }

  .tool-call-body {
    padding: 0 14px 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-height: var(--auxiliary-row-max-height, 280px);
    overflow-y: auto;
    overscroll-behavior: contain;
  }

  .auxiliary-card-body {
    padding-right: 6px;
  }

  .thinking-segment {
    padding: 10px 0;
    border-bottom: 1px solid var(--border-subtle);
  }

  .thinking-segment:last-child {
    padding-bottom: 0;
    border-bottom: none;
  }

  .tool-call-card.reasoning-card {
    border-color: color-mix(in srgb, var(--accent-gold) 30%, var(--border-default));
    background: color-mix(in srgb, var(--bg-surface) 90%, var(--accent-gold) 10%);
  }

  .tool-call-card.image-card {
    border-color: color-mix(in srgb, var(--accent-green) 28%, var(--border-default));
    background: color-mix(in srgb, var(--bg-surface) 92%, var(--accent-green) 8%);
  }

  .tool-group-card {
    padding: 0;
  }

  .tool-group-row {
    display: flex;
    flex-direction: column;
  }

  .tool-group-row-header {
    border-radius: 0;
  }

  .tool-group-row-header .tool-call-left {
    align-items: center;
  }

  .tool-group-divider {
    height: 1px;
    margin: 0;
    background: var(--border-subtle);
  }

  .slide-down {
    animation: slideDown 0.2s ease both;
  }

  @keyframes slideDown {
    from {
      opacity: 0;
      max-height: 0;
    }
    to {
      opacity: 1;
      max-height: var(--auxiliary-row-max-height, 280px);
    }
  }

  .tool-detail {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .tool-detail-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .tool-detail-content {
    margin: 0;
    padding: 0;
    border-radius: 0;
    background: transparent;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-secondary);
    white-space: normal;
    word-break: break-word;
    transition: background-color 0.18s ease, color 0.18s ease;
  }

  .error-detail .tool-detail-content {
    color: var(--accent-danger-text);
  }

  .auxiliary-detail {
    overflow-x: hidden;
  }

  .auxiliary-detail :global(p),
  .auxiliary-detail :global(ul),
  .auxiliary-detail :global(ol),
  .auxiliary-detail :global(blockquote),
  .auxiliary-detail :global(pre) {
    margin-top: 0;
  }

  .auxiliary-detail :global(pre) {
    background: color-mix(in srgb, var(--bg-elevated) 82%, transparent);
  }

  .detail-enter {
    animation: detailFadeIn 0.2s ease both;
  }

  @keyframes detailFadeIn {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  /* Image gallery */
  .image-gallery {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }

  .generated-image {
    display: flex;
    flex-direction: column;
    gap: 4px;
    border-radius: 12px;
    overflow: hidden;
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
    animation: fadeSlideIn 0.3s ease both;
  }

  .generated-image img {
    max-width: 400px;
    max-height: 300px;
    object-fit: contain;
    display: block;
  }

  .image-path {
    padding: 4px 10px 6px;
    font-size: 12px;
    color: var(--text-muted);
    font-family: "SF Mono", "Fira Code", "Cascadia Code", monospace;
  }

  /* Suggestions */
  .suggestions-row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    padding-left: 4px;
  }

  .suggestion-chip {
    padding: 7px 14px;
    border-radius: 16px;
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
    color: var(--text-secondary);
    font-size: 13px;
    cursor: pointer;
    transition: all 0.15s ease;
    font-family: inherit;
  }

  .suggestion-chip:hover {
    background: var(--bg-elevated);
    border-color: var(--border-input);
    color: var(--text-primary);
    transform: translateY(-1px);
  }

  /* Turn cost */
  .turn-meta-stack {
    display: flex;
    flex-direction: column;
    gap: 10px;
    align-items: flex-start;
  }

  .turn-cost-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    font-size: 12px;
    color: var(--text-muted);
    width: fit-content;
    max-width: 100%;
    flex-wrap: wrap;
  }

  .inline-turn-cost {
    margin-top: 10px;
  }

  .turn-cost-copy {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    min-width: 0;
  }

  .reflection-trigger {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    transition: color 0.18s ease, opacity 0.18s ease, transform 0.18s ease;
    font: inherit;
    position: relative;
    opacity: 0.9;
    margin-left: 12px;
  }

  .reflection-trigger:hover {
    color: var(--text-primary);
    opacity: 1;
  }

  .reflection-trigger.is-open {
    color: var(--text-primary);
    opacity: 1;
    transform: translateY(-1px);
  }

  .reflection-trigger-dot {
    width: 6px;
    height: 6px;
    border-radius: 999px;
    position: absolute;
    right: -2px;
    bottom: 1px;
    border: 1.5px solid var(--bg-surface);
    background: var(--text-muted);
  }

  .reflection-pill {
    display: inline-flex;
    align-items: center;
    padding: 2px 7px;
    border-radius: 999px;
    border: 1px solid transparent;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.02em;
    text-transform: uppercase;
  }

  .reflection-pill.large {
    padding: 3px 9px;
  }

  .status-completed {
    color: #14532d;
    background: rgba(134, 239, 172, 0.28);
    border-color: rgba(34, 197, 94, 0.28);
  }

  .status-queued {
    color: #92400e;
    background: rgba(253, 230, 138, 0.28);
    border-color: rgba(217, 119, 6, 0.24);
  }

  .status-running,
  .status-loading {
    color: #1d4ed8;
    background: rgba(147, 197, 253, 0.28);
    border-color: rgba(59, 130, 246, 0.24);
  }

  .status-failed {
    color: #991b1b;
    background: rgba(252, 165, 165, 0.25);
    border-color: rgba(239, 68, 68, 0.24);
  }

  .status-missing,
  .status-unknown,
  .status-unloaded {
    color: var(--text-muted);
    background: var(--bg-hover);
    border-color: var(--border-default);
  }

  .reflection-trigger-dot.status-completed {
    background: #16a34a;
    border-color: var(--bg-surface);
  }

  .reflection-trigger-dot.status-queued {
    background: #d97706;
    border-color: var(--bg-surface);
  }

  .reflection-trigger-dot.status-running,
  .reflection-trigger-dot.status-loading {
    background: #2563eb;
    border-color: var(--bg-surface);
  }

  .reflection-trigger-dot.status-failed {
    background: #dc2626;
    border-color: var(--bg-surface);
  }

  .reflection-trigger-dot.status-missing,
  .reflection-trigger-dot.status-unknown,
  .reflection-trigger-dot.status-unloaded {
    background: var(--text-muted);
    border-color: var(--bg-surface);
  }

  .reflection-modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 60;
    background: rgba(15, 23, 42, 0.32);
    backdrop-filter: blur(8px);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    animation: reflection-backdrop-in 0.18s ease-out both;
  }

  .reflection-modal {
    width: min(760px, 100%);
    max-height: min(78vh, 820px);
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
    border-radius: 18px;
    box-shadow: 0 24px 80px rgba(15, 23, 42, 0.24);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: reflection-modal-in 0.22s cubic-bezier(0.22, 1, 0.36, 1) both;
  }

  .reflection-modal-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 18px 20px 14px;
    border-bottom: 1px solid var(--border-default);
    background: var(--bg-surface);
  }

  .reflection-modal-title-wrap {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }

  .reflection-modal-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: none;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    flex-shrink: 0;
  }

  .reflection-modal-close:hover {
    color: var(--text-primary);
  }

  .reflection-modal-body {
    padding: 16px 20px 20px;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 14px;
    background: var(--bg-primary);
  }

  @keyframes reflection-backdrop-in {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  @keyframes reflection-modal-in {
    from {
      opacity: 0;
      transform: translateY(10px) scale(0.985);
    }
    to {
      opacity: 1;
      transform: translateY(0) scale(1);
    }
  }

  .reflection-panel-title-row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }

  .reflection-panel-title {
    font-size: 13px;
    font-weight: 700;
    letter-spacing: 0.01em;
    color: var(--text-primary);
  }

  .reflection-panel-subtitle {
    margin: 0;
    color: var(--text-secondary);
    font-size: 13px;
    line-height: 1.55;
  }

  .reflection-section {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .reflection-loading {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    color: var(--text-secondary);
    font-size: 13px;
  }

  .reflection-section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    color: var(--text-secondary);
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    font-weight: 700;
  }

  .reflection-list {
    display: flex;
    flex-direction: column;
    gap: 0;
    border-top: 1px solid var(--border-subtle);
  }

  .reflection-list-item {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px 0;
    border-bottom: 1px solid var(--border-subtle);
  }

  .reflection-item-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }

  .reflection-item-title {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    color: var(--text-primary);
  }

  .reflection-item-title strong {
    font-size: 13px;
    font-weight: 700;
  }

  .reflection-item-time {
    color: var(--text-muted);
    font-size: 12px;
    white-space: nowrap;
  }

  .reflection-inline-status {
    display: inline-flex;
    align-items: center;
    padding: 2px 6px;
    border-radius: 999px;
    border: 1px solid transparent;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .reflection-inline-status.tool-running {
    color: #1d4ed8;
    background: rgba(147, 197, 253, 0.28);
    border-color: rgba(59, 130, 246, 0.24);
  }

  .reflection-inline-status.tool-completed {
    color: #14532d;
    background: rgba(134, 239, 172, 0.28);
    border-color: rgba(34, 197, 94, 0.28);
  }

  .reflection-inline-status.tool-failed {
    color: #991b1b;
    background: rgba(252, 165, 165, 0.25);
    border-color: rgba(239, 68, 68, 0.24);
  }

  .reflection-detail-block {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-left: 21px;
  }

  .reflection-detail-block.error {
    color: var(--accent-danger-text);
  }

  .reflection-detail-label {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-muted);
  }

  .reflection-detail-body {
    font-size: 13px;
    color: var(--text-primary);
    line-height: 1.58;
  }

  .reflection-empty {
    margin: 0;
    color: var(--text-muted);
    font-size: 13px;
    line-height: 1.5;
    padding-top: 8px;
  }

  .reflection-live-skeleton {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding-top: 16px;
  }

  .reflection-skeleton-line {
    height: 10px;
    border-radius: 999px;
    background: linear-gradient(
      90deg,
      color-mix(in srgb, var(--bg-hover) 88%, transparent) 0%,
      color-mix(in srgb, var(--bg-elevated) 96%, white 4%) 50%,
      color-mix(in srgb, var(--bg-hover) 88%, transparent) 100%
    );
    background-size: 220% 100%;
    animation: reflection-skeleton-shimmer 1.35s ease-in-out infinite;
  }

  .reflection-skeleton-line.short {
    width: 28%;
  }

  .reflection-skeleton-line.medium {
    width: 56%;
  }

  .reflection-skeleton-line.full {
    width: 100%;
  }

  @keyframes reflection-skeleton-shimmer {
    0% {
      background-position: 100% 50%;
    }
    100% {
      background-position: -100% 50%;
    }
  }

  .cost-sep {
    opacity: 0.5;
  }

  /* Input */
  .input-container {
    width: 100%;
    display: flex;
    flex-direction: column;
    padding: 16px 24px 24px;
    background: var(--bg-primary);
    flex-shrink: 0;
    transition:
      padding 0.28s ease,
      transform 0.32s cubic-bezier(0.22, 1, 0.36, 1),
      gap 0.24s ease;
  }

  .input-container.empty-mode {
    width: min(100%, 840px);
    align-self: center;
    gap: 18px;
    margin-block: auto;
    padding: 0 24px;
    transform: none;
  }

  .empty-composer-copy {
    display: flex;
    justify-content: center;
    text-align: center;
    margin-bottom: 24px;
  }

  .empty-composer-title {
    margin: 0;
    font-size: clamp(28px, 4vw, 42px);
    line-height: 1.08;
    letter-spacing: -0.04em;
    font-weight: 700;
    color: var(--text-primary);
  }

  .input-box {
    background: var(--bg-surface);
    border-radius: 20px;
    padding: 16px;
    border: 1px solid transparent;
    box-shadow: var(--shadow-card);
    transition:
      border-color 0.16s ease,
      background-color 0.16s ease,
      border-radius 0.28s ease,
      box-shadow 0.28s ease,
      transform 0.32s cubic-bezier(0.22, 1, 0.36, 1);
  }

  .input-box.empty-mode {
    border-radius: 24px;
    transform: translateY(0);
  }

  .input-box.drag-active {
    border-color: color-mix(in srgb, var(--accent-primary) 50%, var(--border-default));
    background: color-mix(in srgb, var(--accent-primary) 8%, var(--bg-surface));
    box-shadow:
      var(--shadow-card),
      0 0 0 1px color-mix(in srgb, var(--accent-primary) 22%, transparent);
  }

  .input-no-backend-hint {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 12px;
    font-size: 14px;
    color: var(--text-tertiary);
  }

  .composer-file-input {
    display: none;
  }

  .input-textarea {
    width: 100%;
    min-height: 24px;
    max-height: 200px;
    border: none;
    background: transparent;
    font-size: 15px;
    line-height: 1.5;
    color: var(--text-primary);
    resize: none;
    outline: none;
    font-family: inherit;
  }

  .input-textarea::placeholder {
    color: var(--text-muted);
  }

  .composer-attachments {
    margin-top: 12px;
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .composer-attachment-chip {
    width: auto;
    max-width: 100%;
    padding: 8px 10px;
    cursor: pointer;
    transition: border-color 0.16s ease, transform 0.16s ease, background-color 0.16s ease;
  }

  .composer-attachment-chip:hover {
    border-color: var(--border-strong);
    background: var(--bg-elevated);
    transform: translateY(-1px);
  }

  .attachment-chip-remove {
    width: 24px;
    height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--text-secondary);
    flex-shrink: 0;
  }

  .input-toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 10px;
    padding-top: 10px;
  }

  .input-actions-left {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }

  .input-actions-right {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .input-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border-radius: 20px;
    background: var(--bg-input);
    color: var(--text-tertiary);
    font-size: 13px;
    font-weight: 500;
    border: none;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .input-chip.icon-only {
    padding-inline: 9px;
  }

  .input-chip:hover {
    background: var(--bg-elevated);
  }

  .mode-chip {
    border: 1px solid transparent;
  }

  .mode-chip.active {
    background: color-mix(in srgb, var(--accent-primary) 16%, var(--bg-input));
    color: var(--text-primary);
    border-color: color-mix(in srgb, var(--accent-primary) 42%, transparent);
  }

  .mode-modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 95;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: rgba(12, 17, 26, 0.22);
    backdrop-filter: blur(10px);
  }

  .mode-modal {
    width: min(100%, 520px);
    display: flex;
    flex-direction: column;
    border-radius: 18px;
    border: 1px solid var(--border-default);
    background: var(--bg-surface);
    box-shadow: var(--shadow-dropdown);
    overflow: hidden;
  }

  .mode-modal-head {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 16px;
    padding: 16px 18px 14px;
    border-bottom: 1px solid var(--border-default);
  }

  .mode-modal-head h3 {
    margin: 6px 0 0;
    font-size: 18px;
    color: var(--text-primary);
  }

  .mode-modal-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 34px;
    height: 34px;
    border-radius: 999px;
    border: 1px solid var(--border-default);
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    transition: background-color 0.18s ease, color 0.18s ease, border-color 0.18s ease;
  }

  .mode-modal-close:hover {
    color: var(--text-primary);
    border-color: var(--border-strong);
    background: var(--bg-elevated);
  }

  .mode-modal-body {
    display: grid;
    gap: 12px;
    padding: 16px 18px;
    color: var(--text-secondary);
    line-height: 1.65;
  }

  .mode-modal-body p {
    margin: 0;
  }

  .mode-modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    flex-wrap: wrap;
    padding: 14px 18px 16px;
    border-top: 1px solid var(--border-default);
    background: color-mix(in srgb, var(--bg-primary) 44%, transparent);
  }

  .mode-action-btn {
    min-width: 92px;
    border-radius: 10px;
    padding: 10px 16px;
    font-size: 13px;
    font-weight: 600;
    transform: none;
  }

  .mode-action-btn:hover {
    transform: translateY(-1px);
  }

  .send-btn {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: var(--bg-elevated);
    color: var(--text-on-dark);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 16px;
    border: none;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .send-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .send-btn:hover,
  .send-btn.active {
    background: var(--accent-primary);
  }

  .send-btn-secondary {
    background: color-mix(in srgb, var(--bg-elevated) 68%, var(--accent-primary) 32%);
  }

  .send-btn-busy {
    background: color-mix(in srgb, var(--bg-elevated) 58%, var(--accent-primary) 42%);
  }

  .send-btn-stop {
    background: color-mix(in srgb, var(--accent-danger, #d64545) 88%, var(--accent-primary) 12%);
  }

  .spin {
    animation: send-btn-spin 0.9s linear infinite;
  }

  @keyframes send-btn-spin {
    from {
      transform: rotate(0deg);
    }
    to {
      transform: rotate(360deg);
    }
  }

  @media (max-width: 720px) {
    .mode-modal-backdrop {
      padding: 16px;
    }

    .mode-modal {
      border-radius: 16px;
    }

    .mode-modal-actions {
      justify-content: stretch;
    }

    .mode-modal-actions button {
      flex: 1 1 0;
      justify-content: center;
    }
  }
  .aux-group-card {
    display: flex;
    flex-direction: column;
  }
  .aux-scroll-area {
    overflow: visible;
    flex-shrink: 0;
  }
  .aux-scroll-area::-webkit-scrollbar {
    width: 6px;
  }
  .aux-scroll-area::-webkit-scrollbar-track {
    background: transparent;
  }
  .aux-scroll-area::-webkit-scrollbar-thumb {
    background-color: var(--border-default);
    border-radius: 4px;
  }

  .tool-row-copy {
    display: block;
  }

  .tool-row-inline {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    width: 100%;
  }

  .tool-kind-icon {
    color: var(--text-secondary);
  }

  .tool-inline-loader {
    display: inline-flex;
    color: var(--accent-gold);
    flex-shrink: 0;
    animation: spin 1.1s linear infinite;
  }

  .tool-inline-summary {
    min-width: 0;
    flex: 1;
    font-size: 12px;
    line-height: 1.45;
    color: var(--text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    transition: color 0.18s ease;
  }

  .tool-inline-summary::before {
    content: "·";
    margin-right: 6px;
    color: var(--text-muted);
  }

  .live-tool-summary {
    color: var(--text-primary);
  }

  .summary-settle {
    animation: auxiliarySummarySettle 0.26s ease;
  }

  .tool-inline-error {
    color: var(--accent-danger-text);
  }

  @media (max-width: 720px) {
    .user-message-stack {
      width: min(88%, 100%);
    }

    .input-container.empty-mode {
      width: 100%;
      padding: 0 16px;
      margin-block: auto;
    }

    .empty-composer-title {
      font-size: 28px;
    }
  }

  .pending-request-message {
    width: 100%;
  }

  .pending-inline-skeleton {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
    padding: 6px 0 2px;
  }

  .pending-line {
    display: block;
    height: 11px;
    border-radius: 999px;
    background: linear-gradient(
      90deg,
      color-mix(in srgb, var(--bg-elevated) 82%, transparent),
      color-mix(in srgb, white 74%, var(--accent-primary) 26%),
      color-mix(in srgb, var(--bg-elevated) 82%, transparent)
    );
    background-size: 220% 100%;
    animation: pendingLineShimmer 1.55s ease-in-out infinite;
    opacity: 0.82;
  }

  @keyframes pendingLineShimmer {
    0% {
      background-position: 0% 50%;
      opacity: 0.52;
    }
    50% {
      background-position: 100% 50%;
      opacity: 0.95;
    }
    100% {
      background-position: 0% 50%;
      opacity: 0.52;
    }
  }

  @keyframes auxiliarySummarySettle {
    from {
      opacity: 0.45;
      transform: translateY(2px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  /* Context ring button */
  .context-ring-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    position: relative;
    background: none;
    border: none;
    cursor: pointer;
    padding: 4px;
    color: var(--text-secondary);
    flex-shrink: 0;
  }
  .context-ring-btn:hover {
    color: var(--text-primary);
  }

  /* Context stats modal */
  .context-stats-summary {
    font-size: 12px;
    color: var(--text-secondary);
    padding: 4px 0 12px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 8px;
  }
  .context-stats-summary strong {
    color: var(--text-primary);
  }
  .context-stats-grid {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .context-stat-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 0;
    border-bottom: 1px solid var(--border);
  }
  .context-stat-row:last-child {
    border-bottom: none;
  }
  .context-stat-label {
    font-size: 13px;
    color: var(--text-secondary);
  }
  .context-stat-value {
    font-size: 13px;
    font-weight: 500;
    color: var(--text-primary);
  }
  .context-stat-value.warning {
    color: var(--status-error);
  }
  .context-stat-free {
    padding-top: 8px;
    border-top: 2px solid var(--border);
  }
</style>
