<script lang="ts">
  import {
    Bot,
    FileText,
    GitBranch,
    Plus,
    Shield,
    Sparkles,
    Sun,
    WandSparkles
  } from "lucide-svelte";
  import type { SessionDetail, TaskRecord } from "../lib/types";
  import TaskApprovalCard from "./TaskApprovalCard.svelte";

  interface Props {
    session: SessionDetail | null;
    task: TaskRecord | null;
    modelName?: string | null;
    loading: boolean;
    onSendMessage: (content: string) => void;
    onApproveTask: (task: TaskRecord) => void;
    onApproveTaskAlways: (task: TaskRecord) => void;
    onRejectTask: (task: TaskRecord, reason: string) => void;
  }

  let {
    session,
    task,
    modelName = null,
    loading,
    onSendMessage,
    onApproveTask,
    onApproveTaskAlways,
    onRejectTask
  }: Props = $props();

  let draftMessage = $state("");
  let rejectReason = $state("Rejected by user");
  let textareaRef: HTMLTextAreaElement | null = $state(null);

  const hasMessages = $derived(session && session.messages.length > 0);
  const suggestions = [
    "分析当前项目结构并建议改进方案",
    "自动化构建和部署流程",
    "提取并总结所有 PDF 文件的关键信息"
  ];
  const displayModelName = $derived(modelName?.trim() || "未设置模型");

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
</script>

<div class="chat-area">
  {#if loading}
    <div class="loading-state">
      <p>加载中...</p>
    </div>
  {:else if !session}
    <div class="welcome-screen">
      <h1 class="welcome-title">Hi，今天有什么安排？</h1>

      <div class="quick-actions">
        <button class="quick-action-btn" aria-label="灵感"><Sparkles size={18} strokeWidth={2} /></button>
        <button class="quick-action-btn" aria-label="日程"><Sun size={18} strokeWidth={2} /></button>
        <button class="quick-action-btn" aria-label="创作"><WandSparkles size={18} strokeWidth={2} /></button>
        <button class="quick-action-btn" aria-label="模型"><Bot size={18} strokeWidth={2} /></button>
        <button class="quick-action-btn" aria-label="文档"><FileText size={18} strokeWidth={2} /></button>
        <button class="quick-action-btn" aria-label="集成"><GitBranch size={18} strokeWidth={2} /></button>
        <button class="quick-action-btn" aria-label="更多"><Plus size={18} strokeWidth={2} /></button>
      </div>

      <div class="suggestion-chips">
        {#each suggestions as suggestion}
          <button class="suggestion-chip" onclick={() => { draftMessage = suggestion; }}>
            {suggestion}
          </button>
        {/each}
      </div>
    </div>
  {:else}
    <div class="message-list">
      {#if !hasMessages}
        <div class="empty-state">
          <p>开始新的对话</p>
        </div>
      {:else}
        {#each session.messages as message}
          <div class="message {message.role}">
            <div class="message-avatar">
              {message.role === "user" ? "我" : "AI"}
            </div>
            <div class="message-content">
              {message.content}
            </div>
          </div>
        {/each}
      {/if}
    </div>

    {#if task?.pending_approval}
      <TaskApprovalCard
        {task}
        bind:rejectReason
        onApprove={() => task && onApproveTask(task)}
        onApproveAlways={() => task && onApproveTaskAlways(task)}
        onReject={() => task && onRejectTask(task, rejectReason)}
      />
    {/if}
  {/if}

  <div class="input-container">
    <div class="input-box">
      <textarea
        bind:this={textareaRef}
        bind:value={draftMessage}
        onkeydown={handleKeydown}
        oninput={autoResize}
        class="input-textarea"
        placeholder={session ? `发送消息到 ${displayModelName}` : "Cowork, 发消息、上传文件、打开文件夹或创建定时任务..."}
        rows="1"
      ></textarea>

      <div class="input-toolbar">
        <div class="input-actions-left">
          <button class="input-chip icon-only" aria-label="添加">
            <Plus size={15} strokeWidth={2} />
          </button>
          <button class="input-chip">
            <Bot size={15} strokeWidth={2} />
            <span>{displayModelName}</span>
          </button>
          <button class="input-chip">
            <Shield size={15} strokeWidth={2} />
            <span>权限 · 全自动</span>
          </button>
          {#if task}
            <button class="input-chip active">
              <Sparkles size={15} strokeWidth={2} />
              <span>Cowork</span>
            </button>
          {/if}
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
    background: #f5f0e8;
    height: 100%;
  }

  .loading-state {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: rgba(61, 61, 61, 0.6);
  }

  .welcome-screen {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 40px;
    text-align: center;
  }

  .welcome-title {
    font-size: 32px;
    font-weight: 600;
    color: #3d3d3d;
    margin-bottom: 24px;
  }

  .quick-actions {
    display: flex;
    gap: 8px;
    margin-bottom: 32px;
  }

  .quick-action-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 44px;
    height: 44px;
    border-radius: 12px;
    background: #e8e4dc;
    color: #5c5c5c;
    font-size: 18px;
    border: none;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .quick-action-btn:hover {
    background: #ddd8ce;
    transform: translateY(-2px);
  }

  .suggestion-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    justify-content: center;
    max-width: 600px;
  }

  .suggestion-chip {
    padding: 10px 16px;
    border-radius: 20px;
    background: #e8e4dc;
    color: #5c5c5c;
    font-size: 13px;
    cursor: pointer;
    border: none;
    transition: all 0.15s ease;
  }

  .suggestion-chip:hover {
    background: #ddd8ce;
  }

  .message-list {
    flex: 1;
    overflow-y: auto;
    padding: 24px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .empty-state {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: rgba(61, 61, 61, 0.5);
  }

  .message {
    display: flex;
    gap: 12px;
    max-width: 85%;
  }

  .message.user {
    align-self: flex-end;
    flex-direction: row-reverse;
  }

  .message-avatar {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: #e8e4dc;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 12px;
    font-weight: 600;
    color: #5c5c5c;
    flex-shrink: 0;
  }

  .message.user .message-avatar {
    background: #3d3d3d;
    color: white;
  }

  .message-content {
    background: #ffffff;
    padding: 12px 16px;
    border-radius: 16px;
    font-size: 14px;
    line-height: 1.6;
    color: #3d3d3d;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.04);
    white-space: pre-wrap;
  }

  .message.user .message-content {
    background: #e8e4dc;
  }

  .input-container {
    padding: 24px;
    background: #f5f0e8;
  }

  .input-box {
    background: #ffffff;
    border-radius: 16px;
    padding: 16px;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.04);
  }

  .input-textarea {
    width: 100%;
    min-height: 24px;
    max-height: 200px;
    border: none;
    background: transparent;
    font-size: 15px;
    line-height: 1.5;
    color: #3d3d3d;
    resize: none;
    outline: none;
    font-family: inherit;
  }

  .input-textarea::placeholder {
    color: rgba(61, 61, 61, 0.4);
  }

  .input-toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid rgba(0, 0, 0, 0.06);
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
    background: #f5f0e8;
    color: #6b6b6b;
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
    background: #e8e4dc;
  }

  .input-chip.active {
    background: #3d3d3d;
    color: white;
  }

  .send-btn {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: #d1ccc4;
    color: white;
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
    background: #3d3d3d;
  }
</style>
