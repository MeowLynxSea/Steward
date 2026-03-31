<script lang="ts">
  import { onMount } from "svelte";
  import { apiClient } from "./lib/api";
  import { listenForFolderDrops, notify } from "./lib/tauri";
  import type {
    RuntimeEvent,
    SessionDetail,
    SessionMessage,
    SessionSummary,
    SettingsResponse,
    TaskRecord,
    WorkspaceEntry,
    WorkspaceSearchResult
  } from "./lib/types";

  type View = "sessions" | "tasks" | "workspace" | "settings";

  let currentView: View = "sessions";
  let loading = true;
  let error = "";
  let status = "";

  let settings: SettingsResponse = {
    llm_backend: null,
    selected_model: null,
    ollama_base_url: null,
    openai_compatible_base_url: null,
    llm_custom_providers: [],
    llm_builtin_overrides: {}
  };

  let sessions: SessionSummary[] = [];
  let activeSessionId = "";
  let activeSession: SessionDetail | null = null;
  let draftMessage = "";
  let sessionUnsubscribe: (() => void) | null = null;

  let tasks: TaskRecord[] = [];
  let previousTaskStates = new Map<string, string>();

  let workspacePath = "";
  let workspaceEntries: WorkspaceEntry[] = [];
  let workspaceQuery = "";
  let workspaceResults: WorkspaceSearchResult[] = [];

  async function refreshAll() {
    loading = true;
    error = "";
    try {
      const [settingsResponse, sessionsResponse, tasksResponse, treeResponse] = await Promise.all([
        apiClient.getSettings(),
        apiClient.listSessions(),
        apiClient.listTasks(),
        apiClient.getWorkspaceTree()
      ]);

      settings = settingsResponse;
      sessions = sessionsResponse.sessions;
      tasks = tasksResponse.tasks;
      workspaceEntries = treeResponse.entries;
      workspacePath = treeResponse.path;

      if (!activeSessionId && sessions.length > 0) {
        await selectSession(sessions[0].id);
      }
    } catch (cause) {
      error = cause instanceof Error ? cause.message : "Failed to load IronCowork";
    } finally {
      loading = false;
    }
  }

  async function selectSession(id: string) {
    activeSessionId = id;
    activeSession = await apiClient.getSession(id);
    sessionUnsubscribe?.();
    sessionUnsubscribe = apiClient.streamEvents(`/sessions/${id}/stream`, handleSessionEvent);
  }

  function handleSessionEvent(event: RuntimeEvent) {
    if (!activeSession) {
      return;
    }

    if (event.type === "response") {
      activeSession = {
        ...activeSession,
        messages: [
          ...activeSession.messages,
          {
            id: crypto.randomUUID(),
            role: "assistant",
            content: event.content,
            created_at: new Date().toISOString()
          }
        ]
      };
    } else if (event.type === "approval_needed") {
      status = `Approval needed: ${event.tool_name}`;
      void notify("IronCowork needs confirmation", `${event.tool_name}: ${event.description}`);
    } else if (event.type === "error") {
      error = event.message;
    } else if (event.type === "status") {
      status = event.message;
    }
  }

  function updateSetting<K extends keyof SettingsResponse>(key: K, value: string) {
    settings = {
      ...settings,
      [key]: value || null
    };
  }

  async function createSession() {
    const created = await apiClient.createSession("New Session");
    await refreshSessions();
    await selectSession(created.id);
  }

  async function refreshSessions() {
    const response = await apiClient.listSessions();
    sessions = response.sessions;
  }

  async function sendMessage() {
    const content = draftMessage.trim();
    if (!content || !activeSessionId || !activeSession) {
      return;
    }

    const optimistic: SessionMessage = {
      id: crypto.randomUUID(),
      role: "user",
      content,
      created_at: new Date().toISOString()
    };
    activeSession = {
      ...activeSession,
      messages: [...activeSession.messages, optimistic]
    };
    draftMessage = "";
    await apiClient.sendSessionMessage(activeSessionId, content);
    status = "Message queued";
  }

  async function saveSettings() {
    settings = await apiClient.patchSettings({
      llm_backend: settings.llm_backend,
      selected_model: settings.selected_model,
      ollama_base_url: settings.ollama_base_url,
      openai_compatible_base_url: settings.openai_compatible_base_url,
      llm_builtin_overrides: settings.llm_builtin_overrides
    });
    status = "Settings saved";
  }

  async function refreshTasks() {
    const response = await apiClient.listTasks();
    for (const task of response.tasks) {
      const previous = previousTaskStates.get(task.id);
      if (previous && previous !== task.status) {
        if (task.status === "waiting_approval") {
          void notify("Task waiting for approval", task.title);
        }
        if (task.status === "completed") {
          void notify("Task completed", task.title);
        }
      }
      previousTaskStates.set(task.id, task.status);
    }
    tasks = response.tasks;
  }

  async function toggleTaskMode(task: TaskRecord) {
    await apiClient.toggleTaskYolo(task.id, task.mode !== "yolo");
    await refreshTasks();
  }

  async function approveTask(task: TaskRecord) {
    await apiClient.approveTask(task.id, task.pending_operation?.request_id);
    await refreshTasks();
  }

  async function refreshWorkspace(path = workspacePath) {
    const tree = await apiClient.getWorkspaceTree(path);
    workspacePath = tree.path;
    workspaceEntries = tree.entries;
  }

  async function searchWorkspace() {
    if (!workspaceQuery.trim()) {
      workspaceResults = [];
      return;
    }
    const response = await apiClient.searchWorkspace(workspaceQuery.trim());
    workspaceResults = response.results;
  }

  async function indexWorkspace(path: string) {
    const indexed = await apiClient.indexWorkspace(path);
    status = `Indexed ${indexed.path}`;
    await refreshWorkspace();
  }

  onMount(async () => {
    await refreshAll();
    const taskInterval = window.setInterval(() => {
      void refreshTasks();
    }, 5000);
    const unlistenDrops = await listenForFolderDrops(async (path) => {
      await indexWorkspace(path);
    });

    return () => {
      window.clearInterval(taskInterval);
      sessionUnsubscribe?.();
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
      <span>Local-first task workspace</span>
    </div>

    <nav class="nav">
      <button class:active={currentView === "sessions"} on:click={() => (currentView = "sessions")}>Sessions</button>
      <button class:active={currentView === "tasks"} on:click={() => (currentView = "tasks")}>Tasks</button>
      <button class:active={currentView === "workspace"} on:click={() => (currentView = "workspace")}>Workspace</button>
      <button class:active={currentView === "settings"} on:click={() => (currentView = "settings")}>Settings</button>
    </nav>

    <div class="sidebar-foot">
      <p>{status || "Ready"}</p>
      {#if error}
        <p class="error">{error}</p>
      {/if}
    </div>
  </aside>

  <main class="content">
    {#if loading}
      <section class="panel">
        <h1>Loading local workspace…</h1>
      </section>
    {:else if currentView === "sessions"}
      <section class="sessions-layout">
        <div class="panel session-list">
          <div class="section-head">
            <h1>Sessions</h1>
            <button on:click={createSession}>New</button>
          </div>
          {#if sessions.length === 0}
            <p class="muted">No sessions yet.</p>
          {:else}
            <div class="stack">
              {#each sessions as session}
                <button
                  class:active={session.id === activeSessionId}
                  class="session-item"
                  on:click={() => void selectSession(session.id)}
                >
                  <strong>{session.title}</strong>
                  <span>{session.channel} · {session.message_count} msgs</span>
                </button>
              {/each}
            </div>
          {/if}
        </div>

        <div class="panel chat-panel">
          {#if activeSession}
            <div class="section-head">
              <h1>{activeSession.session.title}</h1>
              <span>{activeSession.session.channel}</span>
            </div>

            <div class="chat-stream">
              {#each activeSession.messages as message}
                <article class:assistant={message.role !== "user"} class="message-card">
                  <header>{message.role}</header>
                  <pre>{message.content}</pre>
                </article>
              {/each}
            </div>

            <div class="composer">
              <textarea bind:value={draftMessage} rows="4" placeholder="Send a message to the local agent"></textarea>
              <button on:click={sendMessage}>Send</button>
            </div>
          {:else}
            <p class="muted">Choose a session to start chatting.</p>
          {/if}
        </div>
      </section>
    {:else if currentView === "tasks"}
      <section class="panel">
        <div class="section-head">
          <h1>Tasks</h1>
          <button on:click={() => void refreshTasks()}>Refresh</button>
        </div>

        <div class="stack">
          {#each tasks as task}
            <article class="task-card">
              <div>
                <strong>{task.title}</strong>
                <p>{task.status} · {task.mode}</p>
                {#if task.last_error}
                  <p class="error">{task.last_error}</p>
                {/if}
              </div>
              <div class="task-actions">
                <button on:click={() => void toggleTaskMode(task)}>
                  {task.mode === "yolo" ? "Switch To Ask" : "Switch To Yolo"}
                </button>
                {#if task.status === "waiting_approval" && task.pending_operation}
                  <button on:click={() => void approveTask(task)}>Approve</button>
                {/if}
              </div>
            </article>
          {/each}
        </div>
      </section>
    {:else if currentView === "workspace"}
      <section class="workspace-layout">
        <div class="panel">
          <div class="section-head">
            <h1>Workspace</h1>
            <button on:click={() => void refreshWorkspace()}>Refresh</button>
          </div>

          <div class="inline-form">
            <input bind:value={workspacePath} placeholder="Folder path to index" />
            <button on:click={() => void indexWorkspace(workspacePath)}>Index Folder</button>
          </div>

          <div class="stack compact">
            {#each workspaceEntries as entry}
              <article class="workspace-entry">
                <strong>{entry.path}</strong>
                <span>{entry.is_directory ? "dir" : "file"}</span>
              </article>
            {/each}
          </div>
        </div>

        <div class="panel">
          <div class="section-head">
            <h1>Search</h1>
          </div>

          <div class="inline-form">
            <input bind:value={workspaceQuery} placeholder="Search indexed notes and documents" />
            <button on:click={searchWorkspace}>Search</button>
          </div>

          <div class="stack compact">
            {#each workspaceResults as result}
              <article class="search-result">
                <strong>{result.document_path}</strong>
                <span>score {result.score.toFixed(3)}</span>
                <p>{result.content}</p>
              </article>
            {/each}
          </div>
        </div>
      </section>
    {:else}
      <section class="panel settings-panel">
        <div class="section-head">
          <h1>Settings</h1>
          <button on:click={saveSettings}>Save</button>
        </div>

        <label>
          <span>LLM Backend</span>
          <input value={settings.llm_backend ?? ""} on:input={(event) => updateSetting("llm_backend", (event.currentTarget as HTMLInputElement).value)} placeholder="openai / ollama / openai_compatible" />
        </label>
        <label>
          <span>Selected Model</span>
          <input value={settings.selected_model ?? ""} on:input={(event) => updateSetting("selected_model", (event.currentTarget as HTMLInputElement).value)} placeholder="gpt-4.1 / qwen2.5-coder" />
        </label>
        <label>
          <span>Ollama Base URL</span>
          <input value={settings.ollama_base_url ?? ""} on:input={(event) => updateSetting("ollama_base_url", (event.currentTarget as HTMLInputElement).value)} placeholder="http://127.0.0.1:11434" />
        </label>
        <label>
          <span>OpenAI Compatible Base URL</span>
          <input value={settings.openai_compatible_base_url ?? ""} on:input={(event) => updateSetting("openai_compatible_base_url", (event.currentTarget as HTMLInputElement).value)} placeholder="http://127.0.0.1:11434/v1" />
        </label>
      </section>
    {/if}
  </main>
</div>
