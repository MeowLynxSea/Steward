<script lang="ts">
  import { onMount } from "svelte";
  import { router, type View } from "./lib/router.svelte";
  import { settingsStore } from "./lib/stores/settings.svelte";
  import { sessionsStore } from "./lib/stores/sessions.svelte";
  import { tasksStore } from "./lib/stores/tasks.svelte";
  import { workspaceStore } from "./lib/stores/workspace.svelte";
  import { workbenchStore } from "./lib/stores/workbench.svelte";
  import { listenForFolderDrops } from "./lib/tauri";
  import type { TaskDetail, TaskRecord, TaskTimelineEntry, WorkspaceSearchResult } from "./lib/types";

  let appLoading = $state(true);
  let appError = $state("");
  let draftMessage = $state("");
  let rejectReason = $state("Rejected by user");

  function taskStatusTone(status: string): string {
    switch (status) {
      case "waiting_approval":
        return "warning";
      case "completed":
        return "success";
      case "failed":
      case "rejected":
      case "cancelled":
        return "danger";
      default:
        return "neutral";
    }
  }

  function taskStatusCopy(status: string): string {
    switch (status) {
      case "waiting_approval":
        return "Waiting for approval";
      case "completed":
        return "Completed";
      case "failed":
        return "Failed";
      case "rejected":
        return "Rejected";
      case "cancelled":
        return "Cancelled";
      case "running":
        return "Running";
      default:
        return "Queued";
    }
  }

  function timelineTitle(item: { current_step: { title: string } | null; event: string }): string {
    return item.current_step?.title || item.event;
  }

  function navLabel(view: View): string {
    switch (view) {
      case "sessions":
        return "Workbench";
      case "tasks":
        return "Runs";
      case "workspace":
        return "Workspace";
      case "settings":
        return "Settings";
    }
  }

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

  function nextRunFocus(task: TaskRecord | null): string {
    if (!task) return "Start with a goal in the composer.";
    if (task.pending_approval) return task.pending_approval.summary;
    if (task.current_step?.title) return task.current_step.title;
    if (task.last_error) return task.last_error;
    return "Run state is synced from the backend.";
  }

  function useWorkspaceResult(result: WorkspaceSearchResult) {
    const snippet = `Use workspace context from ${result.document_path}:\n${result.content}`;
    draftMessage = draftMessage.trim()
      ? `${draftMessage.trim()}\n\n${snippet}`
      : snippet;
  }

  async function submitDraft() {
    const content = draftMessage.trim();
    if (!content) return;
    await sessionsStore.sendMessage(content);
    draftMessage = "";
  }

  function recentRuns() {
    return tasksStore.list.slice(0, 6);
  }

  function outlineSteps(detail: TaskDetail | null): string[] {
    if (!detail) return [];

    const titles = detail.timeline
      .map((item) => item.current_step?.title?.trim() ?? "")
      .filter((title) => title.length > 0 && title !== "Queued");

    const deduped: string[] = [];
    for (const title of titles) {
      if (deduped[deduped.length - 1] !== title) {
        deduped.push(title);
      }
    }
    return deduped.slice(-4);
  }

  function recentTimeline(detail: TaskDetail | null): TaskTimelineEntry[] {
    if (!detail) return [];
    return [...detail.timeline].slice(-4).reverse();
  }

  function resultNotes(detail: TaskDetail | null): string[] {
    const metadata = detail?.task.result_metadata;
    if (!metadata || typeof metadata !== "object") return [];

    const notes: string[] = [];
    const noteValue = metadata.notes;
    if (typeof noteValue === "string" && noteValue.trim()) {
      notes.push(noteValue.trim());
    }

    const artifacts = metadata.artifacts;
    if (Array.isArray(artifacts)) {
      for (const artifact of artifacts) {
        if (
          artifact &&
          typeof artifact === "object" &&
          "path" in artifact &&
          typeof artifact.path === "string" &&
          artifact.path.trim()
        ) {
          notes.push(artifact.path.trim());
        }
      }
    }

    if (notes.length === 0) {
      notes.push(JSON.stringify(metadata, null, 2));
    }

    return notes.slice(0, 4);
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

  async function bootstrap() {
    appLoading = true;
    appError = "";
    try {
      await Promise.all([
        settingsStore.fetch(),
        sessionsStore.fetchList(),
        tasksStore.fetch(),
        workspaceStore.fetch(),
        workbenchStore.fetch()
      ]);

      if (!sessionsStore.activeId && sessionsStore.list.length > 0) {
        await sessionsStore.select(sessionsStore.list[0].id);
      }
    } catch (e) {
      appError = e instanceof Error
        ? e.message
        : "Failed to connect to IronCowork backend. Is the server running?";
    } finally {
      appLoading = false;
    }
  }

  function combinedStatus(): string {
    return (
      sessionsStore.status ||
      tasksStore.status ||
      workspaceStore.status ||
      workbenchStore.status ||
      settingsStore.status ||
      "Ready"
    );
  }

  function combinedError(): string {
    return (
      appError ||
      sessionsStore.error ||
      tasksStore.error ||
      workspaceStore.error ||
      workbenchStore.error ||
      settingsStore.error ||
      ""
    );
  }

  onMount(async () => {
    await bootstrap();

    const taskInterval = window.setInterval(() => {
      void tasksStore.refresh();
    }, 5000);

    const unlistenDrops = await listenForFolderDrops(async (path) => {
      await workspaceStore.index(path);
    });

    return () => {
      window.clearInterval(taskInterval);
      tasksStore.dispose();
      sessionsStore.disconnect();
      workspaceStore.dispose();
      void unlistenDrops();
    };
  });
</script>

<svelte:head>
  <title>IronCowork</title>
</svelte:head>

<div class="app-shell">
  <aside class="sidebar">
    <div class="brand">
      <p>IronCowork</p>
      <span>Session-first local agent workbench</span>
    </div>

    <nav class="nav">
      {#each (["sessions", "tasks", "workspace", "settings"] as View[]) as view}
        <button
          class:active={router.current === view}
          onclick={() => router.navigate(view)}
        >
          {navLabel(view)}
        </button>
      {/each}
    </nav>

    <div class="sidebar-foot">
      <p>{combinedStatus()}</p>
      {#if combinedError()}
        <p class="error">{combinedError()}</p>
      {/if}
    </div>
  </aside>

  <main class="content">
    {#if appLoading}
      <section class="panel">
        <h1>Loading local workbench...</h1>
        <p class="muted">Fetching sessions, runs, workspace context, and supervision data.</p>
      </section>
    {:else if router.current === "sessions"}
      <section class="workbench-layout">
        <div class="panel session-list">
          <div class="section-head">
            <h1>Sessions</h1>
            <button onclick={() => void sessionsStore.create()}>New</button>
          </div>

          {#if sessionsStore.listLoading}
            <p class="muted">Loading sessions...</p>
          {:else if sessionsStore.list.length === 0}
            <p class="muted">No sessions yet. Create one and send a broad goal.</p>
          {:else}
            <div class="stack">
              {#each sessionsStore.list as session}
                <button
                  class:active={session.id === sessionsStore.activeId}
                  class="session-item"
                  onclick={() => void sessionsStore.select(session.id)}
                >
                  <strong>{session.title}</strong>
                  <span>{session.channel} · {session.message_count} msgs</span>
                </button>
              {/each}
            </div>
          {/if}
        </div>

        <div class="panel chat-panel">
          {#if sessionsStore.loading}
            <p class="muted">Loading session...</p>
          {:else if sessionsStore.active}
            <div class="section-head">
              <h1>{sessionsStore.active.session.title}</h1>
              <span>{sessionsStore.active.session.channel}</span>
            </div>

            {@const sessionTask = currentSessionTask()}
            {@const sessionTaskDetail = currentSessionTaskDetail()}
            <article class={`status-banner ${taskStatusTone(sessionTask?.status ?? "queued")}`}>
              <strong>{sessionTask ? taskStatusCopy(sessionTask.status) : "Ready for a new goal"}</strong>
              <span>{nextRunFocus(sessionTask)}</span>
              {#if sessionTask}
                <button
                  onclick={() => {
                    router.navigate("tasks");
                    void tasksStore.select(sessionTask.id);
                  }}
                >
                  Open Run
                </button>
              {/if}
            </article>

            {#if sessionTask}
              <section class="session-visibility">
                <div class="section-head">
                  <h1>Agent Visibility</h1>
                  <div class="task-actions">
                    <button onclick={() => void toggleSessionTaskMode(sessionTask)}>
                      {sessionTask.mode === "yolo" ? "Switch To Ask" : "Switch To Yolo"}
                    </button>
                    {#if sessionTask.status !== "completed" && sessionTask.status !== "failed" && sessionTask.status !== "rejected" && sessionTask.status !== "cancelled"}
                      <button onclick={() => void cancelSessionTask(sessionTask)}>Cancel</button>
                    {/if}
                  </div>
                </div>

                {#if sessionsStore.activeTaskLoading && !sessionTaskDetail}
                  <p class="muted">Loading live run detail for this session...</p>
                {:else}
                  <div class="visibility-grid">
                    <article class="visibility-card">
                      <span class="section-label">Current Action</span>
                      <strong>{sessionTask.current_step?.title ?? "Waiting for the next agent step"}</strong>
                      <p>{taskStatusCopy(sessionTask.status)} · {sessionTask.mode.toUpperCase()}</p>
                    </article>

                    <article class="visibility-card">
                      <span class="section-label">Execution Outline</span>
                      {#if outlineSteps(sessionTaskDetail).length > 0}
                        <div class="outline-list">
                          {#each outlineSteps(sessionTaskDetail) as step}
                            <span>{step}</span>
                          {/each}
                        </div>
                      {:else}
                        <p>No execution outline yet. The run will populate this as state advances.</p>
                      {/if}
                    </article>

                    <article class="visibility-card">
                      <span class="section-label">Result Snapshot</span>
                      {#if resultNotes(sessionTaskDetail).length > 0}
                        <div class="outline-list">
                          {#each resultNotes(sessionTaskDetail) as note}
                            <span>{note}</span>
                          {/each}
                        </div>
                      {:else if sessionTask.last_error}
                        <p>{sessionTask.last_error}</p>
                      {:else}
                        <p>No final output yet. Result metadata will appear here after completion.</p>
                      {/if}
                    </article>
                  </div>

                  {#if sessionTask.pending_approval}
                    <article class="message-card assistant">
                      <header>Pending Approval</header>
                      <p class="approval-summary">{sessionTask.pending_approval.summary}</p>
                      <div class="stack compact">
                        {#each sessionTask.pending_approval.operations as operation, index}
                          <article class="operation-card">
                            <div class="operation-head">
                              <strong>#{index + 1} {operation.kind}</strong>
                              <span>{operation.tool_name}</span>
                            </div>
                            <div class="operation-paths">
                              <span>{operation.path ?? "Unknown source"}</span>
                              <span>{operation.destination_path ?? "No destination"}</span>
                            </div>
                          </article>
                        {/each}
                      </div>

                      <div class="task-actions">
                        <button onclick={() => void approveSessionTask(sessionTask)}>Approve</button>
                        <button onclick={() => void rejectSessionTask(sessionTask)}>Reject</button>
                      </div>

                      <label class="approval-form">
                        <span>Reject reason</span>
                        <textarea bind:value={rejectReason} rows="3" placeholder="Explain why this run should stop"></textarea>
                      </label>
                    </article>
                  {/if}

                  {#if recentTimeline(sessionTaskDetail).length > 0}
                    <article class="message-card assistant">
                      <header>Recent Run Timeline</header>
                      <div class="stack compact">
                        {#each recentTimeline(sessionTaskDetail) as item}
                          <article class="timeline-card compact">
                            <div class="operation-head">
                              <strong>{timelineTitle(item)}</strong>
                              <span>{item.mode}</span>
                            </div>
                            <span>{item.event} · {item.status} · {new Date(item.created_at).toLocaleString()}</span>
                            {#if item.last_error}
                              <p>{item.last_error}</p>
                            {/if}
                          </article>
                        {/each}
                      </div>
                    </article>
                  {/if}
                {/if}
              </section>
            {/if}

            {#if sessionsStore.active.messages.length === 0}
              <p class="muted">Describe a desktop knowledge-work goal and the agent will attach a run to this session.</p>
            {:else}
              <div class="chat-stream">
                {#each sessionsStore.active.messages as message}
                  <article class:assistant={message.role !== "user"} class="message-card">
                    <header>{message.role}</header>
                    <pre>{message.content}</pre>
                  </article>
                {/each}
              </div>
            {/if}

            <div class="composer">
              <div class="inline-form">
                <select bind:value={sessionsStore.messageMode}>
                  <option value="ask">Ask</option>
                  <option value="yolo">Yolo</option>
                </select>
              </div>
              <textarea bind:value={draftMessage} rows="5" placeholder="Describe the goal, constraints, files to inspect, or the summary you want back"></textarea>
              <button onclick={() => void submitDraft()}>Send</button>
            </div>
          {:else}
            <p class="muted">Choose a session to start supervising work.</p>
          {/if}
        </div>

        <div class="workbench-rail">
          <section class="panel rail-panel">
            <div class="section-head">
              <h1>Workspace Context</h1>
              <button onclick={() => void workspaceStore.refresh()}>Refresh</button>
            </div>

            <div class="inline-form">
              <input bind:value={workspaceStore.path} placeholder="Folder path to index" />
              <button onclick={() => void workspaceStore.index(workspaceStore.path)}>Index</button>
            </div>

            {#if workspaceStore.indexJob}
              <article class="status-banner neutral">
                <strong>Index {workspaceStore.indexJob.phase}</strong>
                <span>{workspaceStore.indexJob.processed_files} / {workspaceStore.indexJob.total_files || "?"} files · {workspaceStore.indexJob.indexed_files} indexed · {workspaceStore.indexJob.skipped_files} skipped</span>
              </article>
            {/if}

            <div class="inline-form">
              <input bind:value={workspaceStore.searchQuery} placeholder="Search notes, docs, and workspace memory" />
              <button onclick={() => void workspaceStore.search(workspaceStore.searchQuery)}>Search</button>
            </div>

            {#if workspaceStore.searchResults.length > 0}
              <div class="stack compact">
                {#each workspaceStore.searchResults.slice(0, 4) as result}
                  <article class="search-result">
                    <div class="operation-head">
                      <strong>{result.document_path}</strong>
                      <button onclick={() => useWorkspaceResult(result)}>Use In Prompt</button>
                    </div>
                    {#if result.source_path}
                      <span>{result.source_path}</span>
                    {/if}
                    <p>{result.content}</p>
                  </article>
                {/each}
              </div>
            {:else if workspaceStore.entries.length > 0}
              <div class="stack compact">
                {#each workspaceStore.entries.slice(0, 6) as entry}
                  <article class="workspace-entry">
                    <strong>{entry.path}</strong>
                    <span>{entry.is_directory ? "dir" : "file"}</span>
                  </article>
                {/each}
              </div>
            {:else}
              <p class="muted">Index a folder or search the workspace to ground the current session.</p>
            {/if}
          </section>

          <section class="panel rail-panel">
            <div class="section-head">
              <h1>Capabilities</h1>
              <button onclick={() => void workbenchStore.fetch()}>Refresh</button>
            </div>

            {#if workbenchStore.capabilities}
              <div class="stack compact task-facts">
                <article class="workspace-entry">
                  <strong>Workspace</strong>
                  <span>{workbenchStore.capabilities.workspace_available ? "connected" : "offline"}</span>
                </article>
                <article class="workspace-entry">
                  <strong>Tools</strong>
                  <span>{workbenchStore.capabilities.tool_count}</span>
                </article>
                <article class="workspace-entry">
                  <strong>MCP Servers</strong>
                  <span>{workbenchStore.capabilities.mcp_servers.length}</span>
                </article>
              </div>

              {#if workbenchStore.capabilities.dev_loaded_tools.length > 0}
                <div class="stack compact">
                  {#each workbenchStore.capabilities.dev_loaded_tools.slice(0, 6) as toolName}
                    <article class="workspace-entry">
                      <strong>{toolName}</strong>
                      <span>dev tool</span>
                    </article>
                  {/each}
                </div>
              {/if}

              {#if workbenchStore.capabilities.mcp_servers.length > 0}
                <div class="stack compact">
                  {#each workbenchStore.capabilities.mcp_servers as server}
                    <article class="workspace-entry">
                      <strong>{server.name}</strong>
                      <span>{server.transport} · {server.auth_mode} · {server.enabled ? "enabled" : "disabled"}</span>
                    </article>
                  {/each}
                </div>
              {/if}
            {:else}
              <p class="muted">Capability snapshot is unavailable.</p>
            {/if}
          </section>

          <section class="panel rail-panel">
            <div class="section-head">
              <h1>Run Supervision</h1>
              <button onclick={() => router.navigate("tasks")}>Open Runs</button>
            </div>

            {#if tasksStore.pendingApprovals.length > 0}
              <div class="section-label">Approvals</div>
              <div class="stack compact">
                {#each tasksStore.pendingApprovals.slice(0, 4) as task}
                  <button
                    class="session-item approval-item"
                    onclick={() => {
                      router.navigate("tasks");
                      void tasksStore.select(task.id);
                    }}
                  >
                    <strong>{task.title}</strong>
                    <span>{task.pending_approval?.risk ?? "approval"} · {task.mode}</span>
                  </button>
                {/each}
              </div>
            {/if}

            <div class="section-label">Recent Runs</div>
            <div class="stack compact">
              {#each recentRuns() as task}
                <button
                  class="session-item"
                  onclick={() => {
                    router.navigate("tasks");
                    void tasksStore.select(task.id);
                  }}
                >
                  <strong>{task.title}</strong>
                  <span>{task.status} · {task.mode}</span>
                </button>
              {/each}
            </div>
          </section>
        </div>
      </section>
    {:else if router.current === "tasks"}
      <section class="sessions-layout">
        <div class="panel session-list">
          <div class="section-head">
            <h1>Runs</h1>
            <button onclick={() => void tasksStore.refresh()}>Refresh</button>
          </div>

          {#if tasksStore.pendingApprovals.length > 0}
            <section class="stack compact approval-center">
              <div class="section-label">Approval Center</div>
              {#each tasksStore.pendingApprovals as task}
                <button
                  class:active={task.id === tasksStore.activeId}
                  class="session-item approval-item"
                  onclick={() => void tasksStore.select(task.id)}
                >
                  <strong>{task.title}</strong>
                  <span>{task.pending_approval?.risk ?? "approval"} · {task.mode}</span>
                </button>
              {/each}
            </section>
          {/if}

          {#if tasksStore.loading}
            <p class="muted">Loading runs...</p>
          {:else if tasksStore.list.length === 0}
            <p class="muted">No runs yet. Start from the session workbench.</p>
          {:else}
            <div class="stack">
              {#each tasksStore.list as task}
                <button
                  class:active={task.id === tasksStore.activeId}
                  class="session-item"
                  onclick={() => void tasksStore.select(task.id)}
                >
                  <strong>{task.title}</strong>
                  <span>{task.status} · {task.mode}</span>
                </button>
              {/each}
            </div>
          {/if}

          {#if tasksStore.recentDecisions.length > 0}
            <section class="stack compact decision-center">
              <div class="section-label">Recent Decisions</div>
              {#each tasksStore.recentDecisions.slice(0, 4) as task}
                <button
                  class:active={task.id === tasksStore.activeId}
                  class="session-item decision-item"
                  onclick={() => void tasksStore.select(task.id)}
                >
                  <strong>{task.title}</strong>
                  <span>{task.status} · {task.mode}</span>
                </button>
              {/each}
            </section>
          {/if}
        </div>

        <div class="panel chat-panel">
          {#if tasksStore.detailLoading}
            <p class="muted">Loading run detail...</p>
          {:else if tasksStore.detail}
            <div class="section-head">
              <h1>{tasksStore.detail.task.title}</h1>
              <span>{tasksStore.detail.task.status} · {tasksStore.detail.task.mode}</span>
            </div>

            <div class="task-actions">
              <button onclick={() => void tasksStore.toggleMode(tasksStore.detail!.task)}>
                {tasksStore.detail.task.mode === "yolo" ? "Switch To Ask" : "Switch To Yolo"}
              </button>
              {#if tasksStore.detail.task.status !== "completed" && tasksStore.detail.task.status !== "failed" && tasksStore.detail.task.status !== "rejected" && tasksStore.detail.task.status !== "cancelled"}
                <button onclick={() => void tasksStore.cancel(tasksStore.detail!.task)}>Cancel</button>
              {/if}
              {#if tasksStore.detail.task.status === "waiting_approval" && tasksStore.detail.task.pending_approval}
                <button onclick={() => void tasksStore.approve(tasksStore.detail!.task)}>Approve</button>
                <button onclick={() => void tasksStore.reject(tasksStore.detail!.task, rejectReason)}>Reject</button>
              {/if}
            </div>

            <article class={`status-banner ${taskStatusTone(tasksStore.detail.task.status)}`}>
              <strong>{taskStatusCopy(tasksStore.detail.task.status)}</strong>
              <span>{tasksStore.detail.task.current_step?.title ?? "Run state updated"}</span>
            </article>

            <div class="stack compact task-facts">
              <article class="workspace-entry">
                <strong>Record</strong>
                <span>{tasksStore.detail.task.id}</span>
              </article>
              <article class="workspace-entry">
                <strong>Mode</strong>
                <span>{tasksStore.detail.task.mode}</span>
              </article>
              <article class="workspace-entry">
                <strong>Updated</strong>
                <span>{new Date(tasksStore.detail.task.updated_at).toLocaleString()}</span>
              </article>
            </div>

            {#if tasksStore.detail.task.pending_approval}
              <article class="message-card assistant">
                <header>Approval Preview</header>
                <p class="approval-summary">{tasksStore.detail.task.pending_approval.summary}</p>
                <div class="stack compact">
                  {#each tasksStore.detail.task.pending_approval.operations as operation, index}
                    <article class="operation-card">
                      <div class="operation-head">
                        <strong>#{index + 1} {operation.kind}</strong>
                        <span>{operation.tool_name}</span>
                      </div>
                      <div class="operation-paths">
                        <span>{operation.path ?? "Unknown source"}</span>
                        <span>{operation.destination_path ?? "No destination"}</span>
                      </div>
                    </article>
                  {/each}
                </div>

                <label class="approval-form">
                  <span>Reject reason</span>
                  <textarea bind:value={rejectReason} rows="3" placeholder="Explain why this run should stop"></textarea>
                </label>
              </article>
            {/if}

            {#if tasksStore.detail.task.status === "rejected" || tasksStore.detail.task.status === "failed"}
              <article class="message-card assistant">
                <header>{tasksStore.detail.task.status === "rejected" ? "Rejection Reason" : "Failure Reason"}</header>
                <p>{tasksStore.detail.task.last_error ?? "No reason recorded."}</p>
              </article>
            {/if}

            {#if tasksStore.detail.task.result_metadata}
              <article class="message-card assistant">
                <header>Result Metadata</header>
                <pre>{JSON.stringify(tasksStore.detail.task.result_metadata, null, 2)}</pre>
              </article>
            {/if}

            <div class="stack compact">
              {#each tasksStore.detail.timeline as item}
                <article class="timeline-card">
                  <div class="operation-head">
                    <strong>{timelineTitle(item)}</strong>
                    <span>{item.mode}</span>
                  </div>
                  <span>{item.event} · {item.status} · {new Date(item.created_at).toLocaleString()}</span>
                  {#if item.last_error}
                    <p>{item.last_error}</p>
                  {/if}
                </article>
              {/each}
            </div>
          {:else}
            <p class="muted">Select a run to inspect its timeline and result.</p>
          {/if}
        </div>
      </section>
    {:else if router.current === "workspace"}
      <section class="workspace-layout">
        <div class="panel">
          <div class="section-head">
            <h1>Workspace</h1>
            <button onclick={() => void workspaceStore.refresh()}>Refresh</button>
          </div>

          <div class="inline-form">
            <input bind:value={workspaceStore.path} placeholder="Folder path to index" />
            <button onclick={() => void workspaceStore.index(workspaceStore.path)}>Index Folder</button>
          </div>

          {#if workspaceStore.indexJob}
            <article class="status-banner neutral">
              <strong>Index {workspaceStore.indexJob.phase}</strong>
              <span>{workspaceStore.indexJob.processed_files} / {workspaceStore.indexJob.total_files || "?"} files · {workspaceStore.indexJob.indexed_files} indexed · {workspaceStore.indexJob.skipped_files} skipped</span>
            </article>
          {/if}

          {#if workspaceStore.loading}
            <p class="muted">Loading workspace...</p>
          {:else if workspaceStore.entries.length === 0}
            <p class="muted">Workspace is empty. Index a folder to get started.</p>
          {:else}
            <div class="stack compact">
              {#each workspaceStore.entries as entry}
                <article class="workspace-entry">
                  <strong>{entry.path}</strong>
                  <span>{entry.is_directory ? "dir" : "file"}</span>
                </article>
              {/each}
            </div>
          {/if}
        </div>

        <div class="panel">
          <div class="section-head">
            <h1>Search</h1>
          </div>

          <div class="inline-form">
            <input bind:value={workspaceStore.searchQuery} placeholder="Search indexed notes and documents" />
            <button onclick={() => void workspaceStore.search(workspaceStore.searchQuery)}>Search</button>
          </div>

          {#if workspaceStore.searchLoading}
            <p class="muted">Searching...</p>
          {:else if workspaceStore.searchResults.length === 0}
            <p class="muted">No search results yet. Enter a query above.</p>
          {:else}
            <div class="stack compact">
              {#each workspaceStore.searchResults as result}
                <article class="search-result">
                  <strong>{result.document_path}</strong>
                  <span>score {result.score.toFixed(3)}</span>
                  {#if result.source_path}
                    <span>{result.source_path}</span>
                  {/if}
                  <p>{result.content}</p>
                </article>
              {/each}
            </div>
          {/if}
        </div>
      </section>
    {:else if router.current === "settings"}
      <section class="panel settings-panel">
        <div class="section-head">
          <h1>Settings</h1>
          <button onclick={() => void settingsStore.save()}>Save</button>
        </div>

        {#if settingsStore.loading}
          <p class="muted">Loading settings...</p>
        {:else}
          <label>
            <span>LLM Backend</span>
            <input
              value={settingsStore.data.llm_backend ?? ""}
              oninput={(event) => settingsStore.updateField("llm_backend", (event.currentTarget as HTMLInputElement).value)}
              placeholder="openai / ollama / openai_compatible"
            />
          </label>
          <label>
            <span>Selected Model</span>
            <input
              value={settingsStore.data.selected_model ?? ""}
              oninput={(event) => settingsStore.updateField("selected_model", (event.currentTarget as HTMLInputElement).value)}
              placeholder="gpt-4.1 / qwen2.5-coder"
            />
          </label>
          <label>
            <span>Ollama Base URL</span>
            <input
              value={settingsStore.data.ollama_base_url ?? ""}
              oninput={(event) => settingsStore.updateField("ollama_base_url", (event.currentTarget as HTMLInputElement).value)}
              placeholder="http://127.0.0.1:11434"
            />
          </label>
          <label>
            <span>OpenAI Compatible Base URL</span>
            <input
              value={settingsStore.data.openai_compatible_base_url ?? ""}
              oninput={(event) => settingsStore.updateField("openai_compatible_base_url", (event.currentTarget as HTMLInputElement).value)}
              placeholder="http://127.0.0.1:11434/v1"
            />
          </label>
        {/if}
      </section>
    {/if}
  </main>
</div>
