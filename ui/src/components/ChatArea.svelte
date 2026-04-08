<script lang="ts">
  import {
    Check,
    ChevronDown,
    ChevronRight,
    Circle,
    Loader,
    Moon,
    Plus,
    Shield,
    Sun,
    Wrench,
    Zap,
    Image,
    Brain,
    Sparkles
  } from "lucide-svelte";
  import type {
    SessionDetail,
    ThreadMessage,
    StreamingState,
    TaskRecord,
    TimelineToolCall
  } from "../lib/types";
  import { renderMarkdown } from "../lib/markdown";
  import { themeStore } from "../lib/stores/theme.svelte";
  import TaskApprovalCard from "./TaskApprovalCard.svelte";
  import { onDestroy } from "svelte";

  type ModelOption = {
    value: string;
    label: string;
    model: string;
  };

  interface Props {
    session: SessionDetail | null;
    task: TaskRecord | null;
    streaming: StreamingState;
    modelName?: string | null;
    selectedModelValue?: string;
    availableModels?: ModelOption[];
    loading: boolean;
    onSendMessage: (content: string) => void;
    onSuggestionClick?: (suggestion: string) => void;
    onApproveTask: (task: TaskRecord) => void;
    onApproveTaskAlways: (task: TaskRecord) => void;
    onRejectTask: (task: TaskRecord, reason: string) => void;
    onSelectModel?: (model: string) => void;
  }

  type DisplayEntry =
    | { kind: "message"; message: ThreadMessage }
    | { kind: "auxiliary_group"; id: string; messages: ThreadMessage[] };

  let {
    session,
    task,
    streaming,
    modelName = null,
    selectedModelValue = "",
    availableModels = [],
    loading,
    onSendMessage,
    onSuggestionClick,
    onApproveTask,
    onApproveTaskAlways,
    onRejectTask,
    onSelectModel
  }: Props = $props();

  let draftMessage = $state("");
  let rejectReason = $state("Rejected by user");
  let textareaRef: HTMLTextAreaElement | null = $state(null);
  let messageListRef: HTMLDivElement | null = $state(null);
  let showModelDropdown = $state(false);
  let expandedToolCalls = $state<Set<string>>(new Set());
  let imagesExpanded = $state(false);
  let animatedAssistantId = $state<string | null>(null);
  let animatedAssistantText = $state("");
  let typingTimer: ReturnType<typeof setTimeout> | null = null;
  let animatedThinkingId = $state<string | null>(null);
  let animatedThinkingText = $state("");
  let thinkingTypingTimer: ReturnType<typeof setTimeout> | null = null;
  let settlingAuxiliarySummaries = $state<Set<string>>(new Set());
  let lastLiveThinkingId = $state<string | null>(null);

  const auxiliarySummaryTimers = new Map<string, ReturnType<typeof setTimeout>>();

  const hasMessages = $derived(session && session.thread_messages.length > 0);
  const hasStreamingContent = $derived(
    streaming.images.length > 0
  );
  const darkMode = $derived(themeStore.mode === "dark");
  const displayModelName = $derived(modelName?.trim() || "MiniMax-M2.7");
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
      if (message.kind === "thinking" || (message.kind === "tool_call" && message.tool_call)) {
        auxBuffer.push(message);
        continue;
      }

      flushAux();
      entries.push({ kind: "message", message });
    }

    flushAux();
    return entries;
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
    if (!streaming.thinking || !session) {
      return null;
    }

    for (let i = session.thread_messages.length - 1; i >= 0; i--) {
      const message = session.thread_messages[i];
      if (message.kind === "thinking") {
        return message.id;
      }
    }

    return null;
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
    void session?.thread_messages.length;
    scrollToBottom();
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

  function handleSubmit() {
    const content = draftMessage.trim();
    if (!content) return;
    onSendMessage(content);
    draftMessage = "";

    if (textareaRef) {
      textareaRef.style.height = "auto";
    }
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      handleSubmit();
    }
  }

  function autoResize() {
    if (textareaRef) {
      textareaRef.style.height = "auto";
      textareaRef.style.height = `${Math.min(textareaRef.scrollHeight, 200)}px`;
    }
  }

  function toggleModelDropdown() {
    showModelDropdown = !showModelDropdown;
  }

  function selectModel(modelValue: string) {
    showModelDropdown = false;
    onSelectModel?.(modelValue);
  }

  function toggleTheme() {
    themeStore.toggle();
  }

  function handleGlobalClick(event: MouseEvent) {
    const target = event.target as HTMLElement;
    if (showModelDropdown && !target.closest(".model-selector")) {
      showModelDropdown = false;
    }
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
  });
</script>

<svelte:window onclick={handleGlobalClick} />

<div class="chat-area">
  <!-- Top Navigation Bar -->
  <div class="chat-topbar">
    <div class="topbar-left">
      <div class="model-selector">
        <button class="model-badge" onclick={toggleModelDropdown}>
          {displayModelName}
          <ChevronDown size={13} strokeWidth={2} />
        </button>
        {#if showModelDropdown}
          <div class="model-dropdown">
            <div class="dropdown-header">选择模型</div>
            {#if availableModels.length > 0}
              {#each availableModels as model}
                <button
                  class="dropdown-item {model.value === selectedModelValue ? 'active' : ''}"
                  onclick={() => selectModel(model.value)}
                >
                  <span>{model.label}</span>
                  {#if model.value === selectedModelValue}
                    <Check size={14} strokeWidth={2} />
                  {/if}
                </button>
              {/each}
            {:else}
              <div class="dropdown-item disabled">
                <span>{displayModelName}</span>
                <Check size={14} strokeWidth={2} />
              </div>
              <div class="dropdown-hint">在设置中配置更多模型</div>
            {/if}
          </div>
        {/if}
      </div>
      {#if session}
        <span class="session-title">{session.session.title}</span>
      {/if}
    </div>
    <div class="topbar-right">
      <button class="topbar-icon" onclick={toggleTheme} aria-label={darkMode ? "切换到亮色模式" : "切换到暗色模式"}>
        {#if darkMode}
          <Moon size={16} strokeWidth={2} />
        {:else}
          <Sun size={16} strokeWidth={2} />
        {/if}
      </button>
      <span class="status-indicator">
        <Circle size={8} fill="#4ade80" strokeWidth={0} />
      </span>
    </div>
  </div>

  <!-- Messages Area -->
  {#if loading}
    <div class="loading-state">
      <div class="loading-spinner"></div>
      <p>加载中...</p>
    </div>
  {:else if !session || (!hasMessages && !hasStreamingContent)}
    <div class="empty-chat">
      <div class="empty-chat-inner">
        <div class="empty-icon">
          <Sparkles size={32} strokeWidth={1.5} />
        </div>
        <p class="empty-hint">开始新的对话</p>
      </div>
    </div>
  {:else}
    <div class="message-list" bind:this={messageListRef}>
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
            <div class="user-bubble">
              {entry.message.content ?? ""}
            </div>
          {:else}
            <div class="assistant-text">
              <div class="assistant-content markdown-body">
                {@html renderMarkdown(displayedAssistantContent(entry.message))}
              </div>
              {#if entry.message.turn_cost}
                <div class="turn-cost-bar inline-turn-cost fade-in">
                  <Zap size={12} strokeWidth={2} />
                  <span>{entry.message.turn_cost.input_tokens.toLocaleString()} in</span>
                  <span class="cost-sep">·</span>
                  <span>{entry.message.turn_cost.output_tokens.toLocaleString()} out</span>
                  {#if entry.message.turn_cost.cost_usd !== "$0.0000"}
                    <span class="cost-sep">·</span>
                    <span>{entry.message.turn_cost.cost_usd}</span>
                  {/if}
                </div>
              {/if}
            </div>
          {/if}
        </div>
      {/each}

      {#if streaming.isStreaming}
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
      bind:rejectReason
      onApprove={() => task && onApproveTask(task)}
      onApproveAlways={() => task && onApproveTaskAlways(task)}
      onReject={() => task && onRejectTask(task, rejectReason)}
    />
  {/if}

  <!-- Input Area -->
  <div class="input-container">
    <div class="input-box">
      <textarea
        bind:this={textareaRef}
        bind:value={draftMessage}
        onkeydown={handleKeydown}
        oninput={autoResize}
        class="input-textarea"
        placeholder="发送消息到 {displayModelName}"
        rows="1"
      ></textarea>

      <div class="input-toolbar">
        <div class="input-actions-left">
          <button class="input-chip icon-only" aria-label="添加">
            <Plus size={15} strokeWidth={2} />
          </button>
          <button class="input-chip">
            <Shield size={15} strokeWidth={2} />
            <span>权限 · 全自动</span>
          </button>
        </div>

        <div class="input-actions-right">
          <button class="send-btn {draftMessage.trim() ? 'active' : ''}" onclick={handleSubmit}>
            ↑
          </button>
        </div>
      </div>
    </div>
  </div>
</div>

<style>
  .chat-area {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--bg-primary);
    height: 100%;
  }

  /* Top Navigation */
  .chat-topbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 24px;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-primary);
    flex-shrink: 0;
  }

  .topbar-left {
    display: flex;
    align-items: center;
    gap: 14px;
  }

  .topbar-right {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .model-selector {
    position: relative;
  }

  .model-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 5px 12px;
    border-radius: 20px;
    background: var(--bg-elevated);
    color: var(--text-secondary);
    font-size: 13px;
    font-weight: 500;
    border: none;
    cursor: pointer;
    transition: background 0.15s ease;
  }

  .model-badge:hover {
    background: var(--bg-active);
  }

  .model-dropdown {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    min-width: 240px;
    background: var(--bg-surface);
    border-radius: 14px;
    box-shadow: var(--shadow-dropdown);
    padding: 6px;
    z-index: 50;
    animation: dropdownIn 0.15s ease;
  }

  @keyframes dropdownIn {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .dropdown-header {
    padding: 8px 12px 6px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .dropdown-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 8px 12px;
    border-radius: 10px;
    background: transparent;
    border: none;
    color: var(--text-primary);
    font-size: 13px;
    cursor: pointer;
    transition: background 0.12s ease;
    text-align: left;
  }

  .dropdown-item:hover {
    background: var(--bg-hover);
  }

  .dropdown-item.active {
    background: var(--bg-active);
    font-weight: 500;
  }

  .dropdown-item.disabled {
    cursor: default;
    opacity: 0.8;
  }

  .dropdown-hint {
    padding: 6px 12px 8px;
    font-size: 12px;
    color: var(--text-muted);
  }

  .session-title {
    font-size: 14px;
    color: var(--text-primary);
    font-weight: 500;
  }

  .topbar-icon {
    width: 32px;
    height: 32px;
    border-radius: 8px;
    background: transparent;
    border: none;
    color: var(--text-tertiary);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s ease;
  }

  .topbar-icon:hover {
    background: var(--bg-hover);
  }

  .status-indicator {
    display: inline-flex;
    align-items: center;
    color: var(--accent-green);
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
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .empty-chat-inner {
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
  }

  .empty-icon {
    color: var(--text-muted);
    opacity: 0.5;
  }

  .empty-hint {
    color: var(--text-muted);
    font-size: 15px;
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
  .turn-cost-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    font-size: 12px;
    color: var(--text-muted);
    width: fit-content;
  }

  .inline-turn-cost {
    margin-top: 10px;
  }

  .cost-sep {
    opacity: 0.5;
  }

  /* Input */
  .input-container {
    padding: 16px 24px 24px;
    background: var(--bg-primary);
    flex-shrink: 0;
  }

  .input-box {
    background: var(--bg-surface);
    border-radius: 20px;
    padding: 16px;
    box-shadow: var(--shadow-card);
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

  .send-btn:hover,
  .send-btn.active {
    background: var(--accent-primary);
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

</style>
