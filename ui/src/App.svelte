<script lang="ts">
  import { onMount } from "svelte";
  import { router, type View } from "./lib/router.svelte";
  import { settingsStore } from "./lib/stores/settings.svelte";
  import { sessionsStore } from "./lib/stores/sessions.svelte";
  import { tasksStore } from "./lib/stores/tasks.svelte";
  import { workspaceStore } from "./lib/stores/workspace.svelte";
  import { listenForFolderDrops } from "./lib/tauri";

  let appLoading = $state(true);
  let appError = $state("");
  let draftMessage = $state("");

  async function bootstrap() {
    appLoading = true;
    appError = "";
    try {
      await Promise.all([
        settingsStore.fetch(),
        sessionsStore.fetchList(),
        tasksStore.fetch(),
        workspaceStore.fetch()
      ]);

      // Auto-select first session if none active.
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
      sessionsStore.disconnect();
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
      {#each (["sessions", "tasks", "workspace", "settings"] as View[]) as view}
        <button
          class:active={router.current === view}
          onclick={() => router.navigate(view)}
        >
          {view.charAt(0).toUpperCase() + view.slice(1)}
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
        <h1>Loading local workspace...</h1>
        <p class="muted">Fetching settings, sessions, tasks, and workspace data.</p>
      </section>
    {:else if router.current === "sessions"}
      <section class="sessions-layout">
        <div class="panel session-list">
          <div class="section-head">
            <h1>Sessions</h1>
            <button onclick={() => void sessionsStore.create()}>New</button>
          </div>

          {#if sessionsStore.listLoading}
            <p class="muted">Loading sessions...</p>
          {:else if sessionsStore.list.length === 0}
            <p class="muted">No sessions yet. Click "New" to create one.</p>
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

            {#if sessionsStore.active.messages.length === 0}
              <p class="muted">No messages yet. Send one below.</p>
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
              <textarea bind:value={draftMessage} rows="4" placeholder="Send a message to the local agent"></textarea>
              <button onclick={() => { sessionsStore.sendMessage(draftMessage); draftMessage = ""; }}>Send</button>
            </div>
          {:else}
            <p class="muted">Choose a session to start chatting.</p>
          {/if}
        </div>
      </section>
    {:else if router.current === "tasks"}
      <section class="panel">
        <div class="section-head">
          <h1>Tasks</h1>
          <button onclick={() => void tasksStore.refresh()}>Refresh</button>
        </div>

        {#if tasksStore.loading}
          <p class="muted">Loading tasks...</p>
        {:else if tasksStore.list.length === 0}
          <p class="muted">No tasks found. Tasks will appear here when created.</p>
        {:else}
          <div class="stack">
            {#each tasksStore.list as task}
              <article class="task-card">
                <div>
                  <strong>{task.title}</strong>
                  <p>{task.status} · {task.mode}</p>
                  {#if task.last_error}
                    <p class="error">{task.last_error}</p>
                  {/if}
                </div>
                <div class="task-actions">
                  <button onclick={() => void tasksStore.toggleMode(task)}>
                    {task.mode === "yolo" ? "Switch To Ask" : "Switch To Yolo"}
                  </button>
                  {#if task.status === "waiting_approval" && task.pending_approval}
                    <button onclick={() => void tasksStore.approve(task)}>Approve</button>
                  {/if}
                </div>
              </article>
            {/each}
          </div>
        {/if}
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
