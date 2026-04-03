<script lang="ts">
  import {
    Check,
    ChevronDown,
    Circle,
    Moon,
    Plus,
    Shield,
    Sun
  } from "lucide-svelte";
  import type { SessionDetail, TaskRecord } from "../lib/types";
  import TaskApprovalCard from "./TaskApprovalCard.svelte";

  interface Props {
    session: SessionDetail | null;
    task: TaskRecord | null;
    modelName?: string | null;
    availableModels?: string[];
    loading: boolean;
    onSendMessage: (content: string) => void;
    onApproveTask: (task: TaskRecord) => void;
    onApproveTaskAlways: (task: TaskRecord) => void;
    onRejectTask: (task: TaskRecord, reason: string) => void;
    onSelectModel?: (model: string) => void;
  }

  let {
    session,
    task,
    modelName = null,
    availableModels = [],
    loading,
    onSendMessage,
    onApproveTask,
    onApproveTaskAlways,
    onRejectTask,
    onSelectModel
  }: Props = $props();

  let draftMessage = $state("");
  let rejectReason = $state("Rejected by user");
  let textareaRef: HTMLTextAreaElement | null = $state(null);
  let showModelDropdown = $state(false);
  let darkMode = $state(false);

  const hasMessages = $derived(session && session.messages.length > 0);
  const displayModelName = $derived(modelName?.trim() || "MiniMax-M2.7");

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
      <p>加载中...</p>
    </div>
  {:else if !session || !hasMessages}
    <div class="empty-chat">
      <div class="empty-chat-inner">
        <p class="empty-hint">开始新的对话</p>
      </div>
    </div>
  {:else}
    <div class="message-list">
      {#each session.messages as message}
        <div class="message {message.role}">
          {#if message.role === "user"}
            <div class="user-bubble">
              {message.content}
            </div>
          {:else}
            <div class="assistant-text">
              {message.content}
            </div>
          {/if}
        </div>
      {/each}
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

  /* Messages */
  .loading-state {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-muted);
  }

  .empty-chat {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .empty-chat-inner {
    text-align: center;
  }

  .empty-hint {
    color: var(--text-muted);
    font-size: 15px;
  }

  .message-list {
    flex: 1;
    overflow-y: auto;
    padding: 24px;
    display: flex;
    flex-direction: column;
    gap: 20px;
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

  .user-bubble {
    max-width: 70%;
    padding: 12px 18px;
    border-radius: 20px 20px 4px 20px;
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-size: 15px;
    line-height: 1.6;
    white-space: pre-wrap;
  }

  .assistant-text {
    max-width: 85%;
    padding: 4px 0;
    color: var(--text-primary);
    font-size: 15px;
    line-height: 1.7;
    white-space: pre-wrap;
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
