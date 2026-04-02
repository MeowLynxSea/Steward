<script lang="ts">
  import { sessionsStore } from "../lib/stores/sessions.svelte";
  import { tasksStore } from "../lib/stores/tasks.svelte";
  import { workspaceStore } from "../lib/stores/workspace.svelte";
  import {
    formatDateTime,
    nextRunFocus,
    recentTimeline,
    timelineTitle
  } from "../lib/presentation";
  import type { TaskRecord, WorkspaceSearchResult } from "../lib/types";
  import StatusBadge from "../components/StatusBadge.svelte";
  import TaskApprovalCard from "../components/TaskApprovalCard.svelte";

  let draftMessage = $state("");
  let rejectReason = $state("Rejected by user");
  const sessionTask = $derived(currentSessionTask());
  const sessionTaskDetail = $derived(currentSessionTaskDetail());

  function currentSessionTask() {
    const active = sessionsStore.active;
    if (!active) return null;
    return tasksStore.list.find((task) => task.id === active.session.id) ?? active.current_task;
  }

  function currentSessionTaskDetail() {
    const activeTask = currentSessionTask();
    if (!activeTask) return null;
    if (sessionsStore.activeTaskDetail?.task.id === activeTask.id) {
      return sessionsStore.activeTaskDetail;
    }
    if (tasksStore.detail?.task.id === activeTask.id) {
      return tasksStore.detail;
    }
    return null;
  }

  function useWorkspaceResult(result: WorkspaceSearchResult) {
    const snippet = `Use workspace context from ${result.document_path}:\n${result.content}`;
    draftMessage = draftMessage.trim() ? `${draftMessage.trim()}\n\n${snippet}` : snippet;
  }

  async function submitDraft() {
    const content = draftMessage.trim();
    if (!content) return;
    await sessionsStore.sendMessage(content);
    draftMessage = "";
  }

  async function approveSessionTask(task: TaskRecord) {
    await tasksStore.approve(task);
    await Promise.all([tasksStore.refresh(), sessionsStore.refreshActiveTaskDetail()]);
  }

  async function rejectSessionTask(task: TaskRecord) {
    await tasksStore.reject(task, rejectReason);
    await Promise.all([tasksStore.refresh(), sessionsStore.refreshActiveTaskDetail()]);
  }

  async function toggleSessionTaskMode(task: TaskRecord) {
    await tasksStore.toggleMode(task);
    await Promise.all([tasksStore.refresh(), sessionsStore.refreshActiveTaskDetail()]);
  }

  async function cancelSessionTask(task: TaskRecord) {
    await tasksStore.cancel(task);
    await Promise.all([tasksStore.refresh(), sessionsStore.refreshActiveTaskDetail()]);
  }

  function sessionSubtitle(): string {
    const active = sessionsStore.active;
    if (!active) return "Open a session to chat with the local agent.";
    return `${active.session.channel} · ${active.messages.length} messages`;
  }
</script>

{#if sessionsStore.list.length === 0 && !sessionsStore.listLoading}
  <section class="startup-screen">
    <div class="startup-card">
      <p class="eyebrow">Startup</p>
      <h2>What do you want to build today?</h2>
      <p class="muted">
        Start a local agent session. The workspace file list will appear on the right after the
        chat opens.
      </p>

      <div class="startup-actions">
        <button class="button button-primary" onclick={() => void sessionsStore.create("New Chat")}>
          New Chat
        </button>
      </div>

      <div class="startup-hints">
        <article class="mini-card">
          <strong>Session = workspace</strong>
          <span>Each chat opens as its own workspace-focused conversation.</span>
        </article>
        <article class="mini-card">
          <strong>Workspace rail</strong>
          <span>Files and search stay visible on the right side of the chat window.</span>
        </article>
      </div>
    </div>
  </section>
{:else}
  <section class="chat-layout">
    <aside class="chat-sidebar panel">
      <div class="card-head">
        <div>
          <p class="eyebrow">Chats</p>
          <h2>Workspaces</h2>
        </div>
        <button class="button button-primary" onclick={() => void sessionsStore.create("New Chat")}>
          New
        </button>
      </div>

      {#if sessionsStore.listLoading}
        <p class="muted">Loading sessions...</p>
      {:else}
        <div class="stack compact">
          {#each sessionsStore.list as session}
            <button
              class={`session-tile ${session.id === sessionsStore.activeId ? "active" : ""}`}
              onclick={() => void sessionsStore.select(session.id)}
            >
              <strong>{session.title}</strong>
              <span>{session.message_count} msgs</span>
              <span>{formatDateTime(session.last_activity)}</span>
            </button>
          {/each}
        </div>
      {/if}
    </aside>

    <section class="chat-main panel">
      {#if sessionsStore.loading}
        <p class="muted">Loading session...</p>
      {:else if sessionsStore.active}
        <header class="chat-header">
          <div>
            <p class="eyebrow">Chat</p>
            <h2>{sessionsStore.active.session.title}</h2>
            <p class="muted">{sessionSubtitle()}</p>
          </div>

          <div class="chat-header-side">
            <StatusBadge status={sessionTask?.status ?? "queued"} />
            {#if sessionTask}
              <span class="muted run-focus">{nextRunFocus(sessionTask)}</span>
            {/if}
          </div>
        </header>

        {#if sessionTask}
          <section class="run-strip">
            <div class="run-strip-copy">
              <strong>{sessionTask.current_step?.title ?? "Waiting for next step"}</strong>
              <span>{sessionTask.mode.toUpperCase()} · {sessionTask.status}</span>
            </div>

            <div class="run-strip-actions">
              <button class="button button-ghost" onclick={() => void toggleSessionTaskMode(sessionTask)}>
                {sessionTask.mode === "yolo" ? "Ask" : "Yolo"}
              </button>
              {#if !["completed", "failed", "rejected", "cancelled"].includes(sessionTask.status)}
                <button class="button button-ghost" onclick={() => void cancelSessionTask(sessionTask)}>
                  Stop
                </button>
              {/if}
            </div>
          </section>
        {/if}

        <div class="message-stream chat-stream">
          {#if sessionsStore.active.messages.length === 0}
            <div class="empty-state">
              <h3>Send the first message</h3>
              <p>Describe the task, constraints, and what outcome you want back.</p>
            </div>
          {:else}
            {#each sessionsStore.active.messages as message}
              <article class={`message-bubble ${message.role === "user" ? "user" : "assistant"}`}>
                <header>
                  <strong>{message.role}</strong>
                  <span>{formatDateTime(message.created_at)}</span>
                </header>
                <pre>{message.content}</pre>
              </article>
            {/each}
          {/if}
        </div>

        {#if sessionTask?.pending_approval}
          <TaskApprovalCard
            task={sessionTask}
            bind:rejectReason
            onApprove={() => void approveSessionTask(sessionTask)}
            onReject={() => void rejectSessionTask(sessionTask)}
          />
        {/if}

        {#if recentTimeline(sessionTaskDetail).length > 0}
          <section class="timeline-row">
            {#each recentTimeline(sessionTaskDetail) as item}
              <article class="mini-card">
                <strong>{timelineTitle(item)}</strong>
                <span>{item.event} · {item.status}</span>
                <span>{formatDateTime(item.created_at)}</span>
              </article>
            {/each}
          </section>
        {/if}

        <div class="composer chat-composer">
          <div class="composer-head">
            <select bind:value={sessionsStore.messageMode}>
              <option value="ask">Ask</option>
              <option value="yolo">Yolo</option>
            </select>
            <span class="muted">Drop a folder into the app window to index it.</span>
          </div>

          <textarea
            bind:value={draftMessage}
            rows="4"
            placeholder="Type a message to start or continue the conversation"
          ></textarea>

          <div class="action-row">
            <button class="button button-primary" onclick={() => void submitDraft()}>Send</button>
          </div>
        </div>
      {:else}
        <div class="empty-state">
          <h3>Select a chat</h3>
          <p>Choose a session from the left column.</p>
        </div>
      {/if}
    </section>

    <aside class="workspace-sidebar panel">
      <div class="card-head">
        <div>
          <p class="eyebrow">Workspace</p>
          <h2>Files</h2>
        </div>
        <button class="button button-ghost" onclick={() => void workspaceStore.refresh()}>Refresh</button>
      </div>

      <div class="inline-form">
        <input bind:value={workspaceStore.path} placeholder="Folder path" />
        <button class="button button-secondary" onclick={() => void workspaceStore.index(workspaceStore.path)}>
          Index
        </button>
      </div>

      <div class="inline-form">
        <input bind:value={workspaceStore.searchQuery} placeholder="Search files" />
        <button class="button button-secondary" onclick={() => void workspaceStore.search(workspaceStore.searchQuery)}>
          Search
        </button>
      </div>

      {#if workspaceStore.indexJob}
        <article class="mini-card">
          <strong>{workspaceStore.indexJob.phase}</strong>
          <span>
            {workspaceStore.indexJob.processed_files} / {workspaceStore.indexJob.total_files || "?"}
            files
          </span>
        </article>
      {/if}

      <div class="workspace-list">
        {#if workspaceStore.searchResults.length > 0}
          {#each workspaceStore.searchResults.slice(0, 10) as result}
            <article class="mini-card workspace-item">
              <div class="mini-card-head">
                <strong>{result.document_path}</strong>
                <button class="button button-link" onclick={() => useWorkspaceResult(result)}>Use</button>
              </div>
              {#if result.source_path}
                <span>{result.source_path}</span>
              {/if}
              <p>{result.content}</p>
            </article>
          {/each}
        {:else if workspaceStore.entries.length > 0}
          {#each workspaceStore.entries as entry}
            <article class="mini-card workspace-item">
              <strong>{entry.path}</strong>
              <span>{entry.is_directory ? "Folder" : "File"}</span>
            </article>
          {/each}
        {:else}
          <p class="muted">No indexed files yet.</p>
        {/if}
      </div>
    </aside>
  </section>
{/if}
