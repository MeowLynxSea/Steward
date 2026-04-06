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
    X,
    Zap,
    Image,
    Brain,
    Sparkles,
    AlertCircle,
    CheckCircle2
  } from "lucide-svelte";
  import type { ActiveToolCall, SessionDetail, StreamingState, TaskRecord } from "../lib/types";
  import { renderMarkdown } from "../lib/markdown";
  import TaskApprovalCard from "./TaskApprovalCard.svelte";
  import { tick } from "svelte";

  interface Props {
    session: SessionDetail | null;
    task: TaskRecord | null;
    streaming: StreamingState;
    modelName?: string | null;
    availableModels?: string[];
    loading: boolean;
    onSendMessage: (content: string) => void;
    onSuggestionClick?: (suggestion: string) => void;
    onApproveTask: (task: TaskRecord) => void;
    onApproveTaskAlways: (task: TaskRecord) => void;
    onRejectTask: (task: TaskRecord, reason: string) => void;
    onSelectModel?: (model: string) => void;
  }

  let {
    session,
    task,
    streaming,
    modelName = null,
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
  let darkMode = $state(false);
  let expandedToolCalls = $state<Set<string>>(new Set());

  const hasMessages = $derived(session && session.thread_messages.length > 0);
  const hasStreamingContent = $derived(streaming.streamingContent.length > 0 || streaming.toolCalls.length > 0 || streaming.thinking);
  const displayModelName = $derived(modelName?.trim() || "MiniMax-M2.7");

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
    void streaming.toolCalls.length;
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

  function toolCallDuration(tool: ActiveToolCall): string {
    if (!tool.completedAt) {
      const elapsed = Date.now() - new Date(tool.startedAt).getTime();
      return `${(elapsed / 1000).toFixed(1)}s`;
    }
    const duration = new Date(tool.completedAt).getTime() - new Date(tool.startedAt).getTime();
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

  function selectModel(model: string) {
    showModelDropdown = false;
    onSelectModel?.(model);
  }

  function toggleTheme() {
    darkMode = !darkMode;
    document.documentElement.setAttribute("data-theme", darkMode ? "dark" : "light");
  }

  function handleGlobalClick(event: MouseEvent) {
    const target = event.target as HTMLElement;
    if (showModelDropdown && !target.closest(".model-selector")) {
      showModelDropdown = false;
    }
  }
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
                  class="dropdown-item {model === modelName ? 'active' : ''}"
                  onclick={() => selectModel(model)}
                >
                  <span>{model}</span>
                  {#if model === modelName}
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
      {#each session?.thread_messages ?? [] as message, idx}
        <div class="message {message.role} fade-in" style="animation-delay: {Math.min(idx * 30, 300)}ms">
          {#if message.role === "user"}
            <div class="user-bubble">
              {message.content}
            </div>
          {:else}
            <div class="assistant-text">
              <div class="assistant-content markdown-body">{@html renderMarkdown(message.content)}</div>
            </div>
          {/if}
        </div>
      {/each}

      <!-- Live streaming area -->
      {#if hasStreamingContent || streaming.isStreaming}
        <div class="message assistant streaming-message fade-in">
          <div class="assistant-text">
            <!-- Thinking indicator -->
            {#if streaming.thinking}
              <div class="thinking-indicator">
                <div class="thinking-dots">
                  <span class="dot"></span>
                  <span class="dot"></span>
                  <span class="dot"></span>
                </div>
                <span class="thinking-label">{streaming.thinkingMessage || "思考中..."}</span>
              </div>
            {/if}

            <!-- Reasoning update -->
            {#if streaming.reasoning}
              <div class="reasoning-block">
                <div class="reasoning-header">
                  <Brain size={14} strokeWidth={2} />
                  <span>推理过程</span>
                </div>
                <p class="reasoning-text">{streaming.reasoning}</p>
                {#if streaming.reasoningDecisions.length > 0}
                  <div class="reasoning-decisions">
                    {#each streaming.reasoningDecisions as decision}
                      <div class="decision-chip">
                        <Wrench size={12} strokeWidth={2} />
                        <span class="decision-tool">{decision.tool_name}</span>
                        <span class="decision-rationale">{decision.rationale}</span>
                      </div>
                    {/each}
                  </div>
                {/if}
              </div>
            {/if}

            <!-- Tool calls -->
            {#if streaming.toolCalls.length > 0}
              <div class="tool-calls-container">
                {#each streaming.toolCalls as tool (tool.id)}
                  <div class="tool-call-card {tool.status}" class:expanded={expandedToolCalls.has(tool.id)}>
                    <button class="tool-call-header" onclick={() => toggleToolCallExpand(tool.id)}>
                      <div class="tool-call-left">
                        {#if tool.status === "running"}
                          <div class="tool-spinner">
                            <Loader size={14} strokeWidth={2} />
                          </div>
                        {:else if tool.status === "completed"}
                          <div class="tool-icon success">
                            <CheckCircle2 size={14} strokeWidth={2} />
                          </div>
                        {:else}
                          <div class="tool-icon error">
                            <AlertCircle size={14} strokeWidth={2} />
                          </div>
                        {/if}
                        <span class="tool-name">{tool.name}</span>
                        <span class="tool-duration">{toolCallDuration(tool)}</span>
                      </div>
                      <div class="tool-call-right">
                        <ChevronRight size={14} strokeWidth={2} class="expand-icon" />
                      </div>
                    </button>
                    {#if expandedToolCalls.has(tool.id)}
                      <div class="tool-call-body slide-down">
                        {#if tool.parameters}
                          <div class="tool-detail">
                            <span class="tool-detail-label">参数</span>
                            <pre class="tool-detail-content">{tool.parameters}</pre>
                          </div>
                        {/if}
                        {#if tool.resultPreview}
                          <div class="tool-detail">
                            <span class="tool-detail-label">结果</span>
                            <pre class="tool-detail-content">{tool.resultPreview}</pre>
                          </div>
                        {/if}
                        {#if tool.error}
                          <div class="tool-detail error-detail">
                            <span class="tool-detail-label">错误</span>
                            <pre class="tool-detail-content">{tool.error}</pre>
                          </div>
                        {/if}
                      </div>
                    {/if}
                  </div>
                {/each}
              </div>
            {/if}

            <!-- Streaming text content -->
            {#if streaming.streamingContent}
              <div class="assistant-content markdown-body streaming-text">
                {@html renderMarkdown(streaming.streamingContent)}<span class="typing-cursor"></span>
              </div>
            {/if}

            <!-- Generated images -->
            {#if streaming.images.length > 0}
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

      <!-- Turn cost -->
      {#if streaming.turnCost && !streaming.isStreaming}
        <div class="turn-cost-bar fade-in">
          <Zap size={12} strokeWidth={2} />
          <span>{streaming.turnCost.input_tokens.toLocaleString()} in</span>
          <span class="cost-sep">·</span>
          <span>{streaming.turnCost.output_tokens.toLocaleString()} out</span>
          {#if streaming.turnCost.cost_usd !== "0.000000"}
            <span class="cost-sep">·</span>
            <span>${streaming.turnCost.cost_usd}</span>
          {/if}
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
    gap: 20px;
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
    justify-content: flex-start;
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
    max-width: 85%;
    padding: 4px 0;
    color: var(--text-primary);
    font-size: 15px;
    line-height: 1.7;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .assistant-content {
    white-space: pre-wrap;
    word-break: break-word;
  }

  /* Streaming text with cursor */
  .streaming-text {
    position: relative;
  }

  .typing-cursor {
    display: inline-block;
    width: 2px;
    height: 1.1em;
    background: var(--accent-primary);
    margin-left: 2px;
    vertical-align: text-bottom;
    animation: cursorBlink 1s step-end infinite;
  }

  @keyframes cursorBlink {
    0%, 100% { opacity: 1; }
    50% { opacity: 0; }
  }

  /* Thinking indicator */
  .thinking-indicator {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 14px;
    border-radius: 12px;
    background: var(--bg-elevated);
    width: fit-content;
    animation: fadeSlideIn 0.25s ease both;
  }

  .thinking-dots {
    display: flex;
    gap: 4px;
    align-items: center;
  }

  .thinking-dots .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text-tertiary);
    animation: dotPulse 1.4s ease-in-out infinite;
  }

  .thinking-dots .dot:nth-child(2) {
    animation-delay: 0.2s;
  }

  .thinking-dots .dot:nth-child(3) {
    animation-delay: 0.4s;
  }

  @keyframes dotPulse {
    0%, 80%, 100% {
      opacity: 0.3;
      transform: scale(0.8);
    }
    40% {
      opacity: 1;
      transform: scale(1);
    }
  }

  .thinking-label {
    font-size: 13px;
    color: var(--text-tertiary);
    font-style: italic;
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

  .tool-call-card {
    border-radius: 12px;
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
    overflow: hidden;
    transition: border-color 0.2s ease, box-shadow 0.2s ease;
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
    align-items: center;
    gap: 8px;
  }

  .tool-spinner {
    color: var(--accent-gold);
    display: flex;
    animation: spin 1.2s linear infinite;
  }

  .tool-icon.success {
    color: var(--accent-green);
    display: flex;
  }

  .tool-icon.error {
    color: var(--accent-danger-text);
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
  }

  .tool-call-right {
    color: var(--text-muted);
    display: flex;
    transition: transform 0.2s ease;
  }

  .tool-call-card.expanded .tool-call-right {
    transform: rotate(90deg);
  }

  .tool-call-body {
    padding: 0 14px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
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
      max-height: 500px;
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
    padding: 8px 10px;
    border-radius: 8px;
    background: var(--bg-elevated);
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-secondary);
    font-family: "SF Mono", "Fira Code", "Cascadia Code", monospace;
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 200px;
    overflow-y: auto;
  }

  .error-detail .tool-detail-content {
    background: var(--accent-danger);
    color: var(--accent-danger-text);
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
</style>
