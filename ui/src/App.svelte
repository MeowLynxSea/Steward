<script lang="ts">
  import { onMount } from "svelte";
  import { fade, scale } from "svelte/transition";
  import ChatArea from "./components/ChatArea.svelte";
  import LeftSidebar from "./components/LeftSidebar.svelte";
  import RightSidebar from "./components/RightSidebar.svelte";
  import TitleBar from "./components/TitleBar.svelte";
  import { FolderPlus, FolderSearch, X } from "lucide-svelte";
  import { router } from "./lib/router.svelte";
  import { settingsStore } from "./lib/stores/settings.svelte";
  import { sessionsStore } from "./lib/stores/sessions.svelte";
  import { themeStore } from "./lib/stores/theme.svelte";
  import { tasksStore } from "./lib/stores/tasks.svelte";
  import { workspaceStore } from "./lib/stores/workspace.svelte";
  import { listenForFolderDrops, pickDirectory } from "./lib/tauri";
  import type { TaskMode, TaskRecord, WorkspaceSearchResult } from "./lib/types";
  import OnboardingView from "./views/OnboardingView.svelte";
  import SettingsView from "./views/SettingsView.svelte";
  import ToastContainer from "./components/ToastContainer.svelte";

  const providerLabels: Record<string, string> = {
    openai: "OpenAI",
    openai_codex: "Codex",
    anthropic: "Anthropic",
    groq: "Groq",
    openrouter: "OpenRouter",
    ollama: "Ollama"
  };

  type ModelOption = {
    value: string;
    label: string;
    model: string;
  };

  let appLoading = $state(true);
  let appError = $state("");
  let leftSidebarCollapsed = $state(false);
  let rightSidebarCollapsed = $state(false);
  let showSettings = $state(false);
  let showAllowlistModal = $state(false);
  let composerSeed = $state<{ id: string; content: string } | null>(null);

  function openSettings() {
    showSettings = true;
  }

  function closeSettings() {
    // Delay to allow fly out animation (220ms) to complete before removing from DOM
    setTimeout(() => {
      showSettings = false;
      router.navigate("sessions");
    }, 250);
  }

  // Sync settings modal state with router
  $effect(() => {
    if (router.current === "settings") {
      showSettings = true;
    }
  });
  let allowlistDisplayName = $state("");
  let selectedAllowlistPath = $state("");
  let selectingAllowlistPath = $state(false);

  async function loadWorkspaceData() {
    await Promise.all([
      sessionsStore.fetchList(),
      tasksStore.fetch(),
      workspaceStore.fetch()
    ]);

    if (!sessionsStore.activeId && sessionsStore.list.length > 0) {
      await sessionsStore.select(sessionsStore.list[0].id);
    }
  }

  async function bootstrap() {
    appLoading = true;
    appError = "";

    try {
      await settingsStore.fetch();

      if (settingsStore.data.llm_ready) {
        await loadWorkspaceData();
      }
    } catch (error) {
      appError = error instanceof Error
        ? error.message
        : "Failed to connect to Steward backend. Is the server running?";
    } finally {
      appLoading = false;
    }
  }

  async function handleOnboardingComplete() {
    if (!settingsStore.data.llm_ready) {
      return;
    }
    await loadWorkspaceData();
    router.navigate("sessions");
  }

  function handleSendMessage(content: string, files: File[]) {
    return sessionsStore.sendMessage(content, files);
  }

  function handleSheerSendMessage(content: string, files: File[]) {
    return sessionsStore.sendSheerMessage(content, files);
  }

  function handleQueueSendMessage(content: string, files: File[]) {
    return sessionsStore.sendQueuedMessage(content, files);
  }

  function handleInterruptSession() {
    return sessionsStore.interruptSession();
  }

  function handleSuggestionClick(suggestion: string) {
    void sessionsStore.sendMessage(suggestion);
  }

  function handleMessageModeChange(mode: TaskMode) {
    void sessionsStore.setMessageMode(mode);
  }

  function handleSelectModel(backendId: string) {
    settingsStore.setMajorBackend(backendId);
    void settingsStore.save();
  }

  const currentMajorBackend = $derived.by(
    () => settingsStore.data.backends.find((backend) => backend.id === settingsStore.data.major_backend_id) ?? null
  );

  const currentModelName = $derived.by(
    () => currentMajorBackend?.model ?? null
  );

  const availableModels = $derived.by<ModelOption[]>(() =>
    settingsStore.data.backends.map((backend) => ({
      value: backend.id,
      label: `${providerLabels[backend.provider] ?? backend.provider} / ${backend.model}`,
      model: backend.model
    }))
  );

  // Auto-select the sole remaining backend when backends.length drops to 1.
  $effect(() => {
    const backends = settingsStore.data.backends;
    if (backends.length === 1 && !settingsStore.data.major_backend_id) {
      settingsStore.setMajorBackend(backends[0].id);
      void settingsStore.save();
    }
  });

  const noBackend = $derived(settingsStore.data.backends.length === 0);
  const prefersEmptySessionLayout = $derived.by(() => {
    const activeSession = sessionsStore.active;
    const streaming = sessionsStore.streaming;
    const hasLiveSessionSignal = Boolean(
      streaming.isStreaming ||
      streaming.thinking ||
      streaming.streamingContent.trim() ||
      streaming.toolCalls.length > 0 ||
      streaming.reasoning ||
      streaming.images.length > 0
    );

    return !sessionsStore.loading && (!activeSession || (activeSession.thread_messages.length === 0 && !hasLiveSessionSignal));
  });
  let lastPrefersEmptySessionLayout = $state<boolean | null>(null);

  $effect(() => {
    if (prefersEmptySessionLayout && lastPrefersEmptySessionLayout !== true) {
      leftSidebarCollapsed = false;
      rightSidebarCollapsed = true;
    }
    if (!prefersEmptySessionLayout && lastPrefersEmptySessionLayout === true) {
      rightSidebarCollapsed = false;
    }

    lastPrefersEmptySessionLayout = prefersEmptySessionLayout;
  });

  function handleApproveTask(task: TaskRecord) {
    void (async () => {
      await sessionsStore.approveTask(task);
      await tasksStore.refresh();
    })();
  }

  function handleApproveTaskAlways(task: TaskRecord) {
    void (async () => {
      await sessionsStore.approveTask(task, true);
      await tasksStore.refresh();
    })();
  }

  function handleRejectTask(task: TaskRecord, reason: string) {
    void (async () => {
      await sessionsStore.rejectTask(task, reason);
      await tasksStore.refresh();
    })();
  }

  function handleUseWorkspaceResult(result: WorkspaceSearchResult) {
    composerSeed = {
      id: crypto.randomUUID(),
      content: `Use workspace context from ${result.document_path}:\n${result.content}`
    };
  }

  function handleSeedComposer(content: string) {
    composerSeed = {
      id: crypto.randomUUID(),
      content
    };
  }

  function openAllowlistModal() {
    showAllowlistModal = true;
  }

  function closeAllowlistModal() {
    showAllowlistModal = false;
    allowlistDisplayName = "";
    selectedAllowlistPath = "";
    selectingAllowlistPath = false;
  }

  async function handlePickAllowlistDirectory() {
    selectingAllowlistPath = true;
    try {
      const selected = await pickDirectory();
      if (selected) {
        selectedAllowlistPath = selected;
        allowlistDisplayName = defaultAllowlistName(selected);
      }
    } finally {
      selectingAllowlistPath = false;
    }
  }

  async function handleCreateAllowlist() {
    if (!selectedAllowlistPath) return;
    await workspaceStore.createAllowlist(selectedAllowlistPath, allowlistDisplayName.trim() || undefined);
    if (!workspaceStore.error) {
      closeAllowlistModal();
    }
  }

  function defaultAllowlistName(path: string) {
    const segments = path.split(/[\\/]/).filter(Boolean);
    return segments.at(-1) ?? path;
  }

  onMount(() => {
    themeStore.init();
    let disposed = false;
    let taskInterval: number | null = null;
    let workspaceInterval: number | null = null;
    let unlistenDrops: (() => void) | null = null;

    void (async () => {
      await bootstrap();
      if (disposed) {
        return;
      }

      taskInterval = window.setInterval(() => {
        void tasksStore.refresh();
      }, 5000);

      workspaceInterval = window.setInterval(() => {
        if (
          showAllowlistModal ||
          workspaceStore.loading ||
          workspaceStore.refreshing ||
          workspaceStore.busyAction
        ) {
          return;
        }
        void workspaceStore.refresh();
      }, 4000);

      unlistenDrops = await listenForFolderDrops(async (path) => {
        await workspaceStore.createAllowlist(path, defaultAllowlistName(path));
      });

      if (disposed) {
        if (taskInterval !== null) {
          window.clearInterval(taskInterval);
        }
        if (workspaceInterval !== null) {
          window.clearInterval(workspaceInterval);
        }
        unlistenDrops();
      }
    })();

    return () => {
      disposed = true;
      if (taskInterval !== null) {
        window.clearInterval(taskInterval);
      }
      if (workspaceInterval !== null) {
        window.clearInterval(workspaceInterval);
      }
      tasksStore.dispose();
      sessionsStore.disconnect();
      workspaceStore.dispose();
      unlistenDrops?.();
    };
  });
</script>

<svelte:head>
  <title>Steward</title>
</svelte:head>

{#if appLoading}
  <div class="loading-screen">
    <div class="loading-content">
      <h2>加载中...</h2>
      <p>正在连接到 Steward 后端</p>
    </div>
  </div>
{:else if appError}
  <div class="error-screen">
    <div class="error-content">
      <h2>连接错误</h2>
      <p>{appError}</p>
      <button class="btn btn-primary" onclick={() => location.reload()}>重试</button>
    </div>
  </div>
{:else if settingsStore.data.llm_onboarding_required}
  <div class="app-container">
    <TitleBar
      title="Steward"
      leftSidebarCollapsed={true}
      rightSidebarCollapsed={true}
      onToggleLeft={() => undefined}
      onToggleRight={() => undefined}
    />
    <div class="onboarding-layout">
      <OnboardingView onComplete={handleOnboardingComplete} />
    </div>
  </div>
{:else}
  <div class="app-container">
    <TitleBar
      title="Steward"
      session={sessionsStore.active?.session ?? null}
      leftSidebarCollapsed={leftSidebarCollapsed}
      rightSidebarCollapsed={rightSidebarCollapsed}
      onToggleLeft={() => leftSidebarCollapsed = !leftSidebarCollapsed}
      onToggleRight={() => rightSidebarCollapsed = !rightSidebarCollapsed}
      availableModels={availableModels}
      selectedModelValue={settingsStore.data.major_backend_id ?? currentModelName ?? ""}
      onSelectModel={handleSelectModel}
      onOpenSettings={openSettings}
    />

    <div class="main-layout">
      <LeftSidebar
        sessions={sessionsStore.list}
        activeId={sessionsStore.activeId}
        collapsed={leftSidebarCollapsed}
        onSelect={(id) => void sessionsStore.select(id)}
        onCreate={() => void sessionsStore.create("新会话")}
        onDelete={(id) => void sessionsStore.delete(id)}
        onSettings={openSettings}
      />

      <div class="center-area">
        <ChatArea
          session={sessionsStore.active}
          runtimeStatus={sessionsStore.runtimeStatus}
          task={sessionsStore.active?.active_thread_task ?? null}
          messageMode={sessionsStore.messageMode}
          streaming={sessionsStore.streaming}
          loading={sessionsStore.loading}
          emptyLayout={prefersEmptySessionLayout}
          noBackend={noBackend}
          {composerSeed}
          onSendMessage={handleSendMessage}
          onSheerSendMessage={handleSheerSendMessage}
          onQueueSendMessage={handleQueueSendMessage}
          onInterruptSession={handleInterruptSession}
          onChangeMessageMode={handleMessageModeChange}
          onSuggestionClick={handleSuggestionClick}
          onApproveTask={handleApproveTask}
          onApproveTaskAlways={handleApproveTaskAlways}
          onRejectTask={handleRejectTask}
        />
      </div>

      <RightSidebar
        currentPath={workspaceStore.currentPath}
        entries={workspaceStore.entries}
        searchResults={workspaceStore.searchResults}
        searchQuery={workspaceStore.searchQuery}
        selectedAllowlist={workspaceStore.selectedAllowlist}
        selectedFile={workspaceStore.selectedFile}
        selectedDocument={workspaceStore.selectedDocument}
        changeGroups={workspaceStore.changeGroups}
        loading={workspaceStore.loading}
        fileLoading={workspaceStore.fileLoading}
        searchLoading={workspaceStore.searchLoading}
        busyAction={workspaceStore.busyAction}
        collapsed={rightSidebarCollapsed}
        onSearch={(query) => void workspaceStore.search(query)}
        onClearSearch={() => workspaceStore.clearSearch()}
        onClearPreview={() => workspaceStore.clearPreview()}
        onRequestAllowlist={openAllowlistModal}
        onNavigate={(path) => void workspaceStore.openPath(path)}
        onOpenEntry={(entry) => void workspaceStore.openEntry(entry)}
        onOpenChangesTab={() => void workspaceStore.refreshAllowlistChanges()}
        onKeepAllowlist={(allowlistId, scopePath, checkpointId) => void workspaceStore.keepAllowlist(allowlistId, scopePath, checkpointId)}
        onRevertAllowlist={(allowlistId, scopePath, checkpointId) => void workspaceStore.revertAllowlist(allowlistId, scopePath, checkpointId)}
        onCreateCheckpoint={(allowlistId, label, summary) => void workspaceStore.createCheckpoint(allowlistId, label, summary)}
        onRestoreCheckpoint={(allowlistId, checkpointId) => void workspaceStore.restoreCheckpoint(allowlistId, checkpointId)}
        onDeleteCheckpoint={(allowlistId, checkpointId) =>
          void workspaceStore.deleteCheckpoint(allowlistId, checkpointId)
        }
        onWriteFile={(path, content) => workspaceStore.writeFile(path, content)}
        onDeleteFile={(path, allowlistId) => workspaceStore.deleteFile(path, allowlistId)}
        onResolveConflict={(allowlistId, path, resolution, renamedCopyPath, mergedContent) =>
          void workspaceStore.resolveConflict(allowlistId, path, resolution, renamedCopyPath, mergedContent)}
        onUseResult={handleUseWorkspaceResult}
      />
    </div>

    {#if showAllowlistModal}
      <div
        class="global-modal-backdrop"
        role="presentation"
        onclick={closeAllowlistModal}
        transition:fade={{ duration: 180 }}
        onkeydown={(event) => event.key === "Escape" && closeAllowlistModal()}
      >
        <div
          class="global-allowlist-modal"
          role="dialog"
          aria-modal="true"
          aria-label="授权目录"
          tabindex="-1"
          in:scale={{ duration: 220, start: 0.92 }}
          out:scale={{ duration: 150, start: 0.96 }}
        >
          <div class="global-allowlist-modal-inner" role="presentation" onclick={(event) => event.stopPropagation()}>
            <div class="allowlist-modal-head">
              <strong>授权目录</strong>
              <button class="modal-icon-button" onclick={closeAllowlistModal} aria-label="关闭">
                <X size={18} strokeWidth={2} />
              </button>
            </div>

            <button class="folder-picker-button" onclick={() => void handlePickAllowlistDirectory()}>
              <FolderSearch size={18} strokeWidth={2} />
              {selectingAllowlistPath ? "正在打开选择器..." : selectedAllowlistPath ? "重新选择文件夹" : "选择文件夹"}
            </button>

            <div class="picked-folder {selectedAllowlistPath ? 'selected' : ''}">
              <span class="picked-folder-label">已选目录</span>
              <span class="picked-folder-path">{selectedAllowlistPath || "尚未选择文件夹"}</span>
            </div>

            <input
              class="allowlist-name-input"
              type="text"
              bind:value={allowlistDisplayName}
              placeholder="显示名称（可选）"
              onkeydown={(event) => event.key === "Enter" && void handleCreateAllowlist()}
            />

            <div class="allowlist-modal-actions">
              <button class="modal-action secondary" onclick={closeAllowlistModal}>取消</button>
              <button
                class="modal-action primary"
                onclick={() => void handleCreateAllowlist()}
                disabled={!selectedAllowlistPath || workspaceStore.loading}
              >
                <FolderPlus size={16} strokeWidth={2} />
                创建授权
              </button>
            </div>

            {#if workspaceStore.error}
              <p class="allowlist-modal-feedback error">{workspaceStore.error}</p>
            {:else if workspaceStore.status}
              <p class="allowlist-modal-feedback">{workspaceStore.status}</p>
            {/if}
          </div>
        </div>
      </div>
    {/if}

    {#if showSettings}
      <SettingsView onClose={closeSettings} onSeedComposer={handleSeedComposer} />
    {/if}

    <ToastContainer />
  </div>
{/if}

<style>
  :global(html) {
    background: transparent;
  }

  :global(body) {
    margin: 0;
    padding: 0;
    font-family: "SF Pro Display", -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    background: transparent;
    color: var(--text-primary);
    overflow: hidden;
  }

  :global(#app) {
    width: 100vw;
    height: 100vh;
    overflow: hidden;
    background: transparent;
  }

  .app-container {
    display: flex;
    flex-direction: column;
    position: relative;
    width: 100vw;
    height: 100vh;
    overflow: hidden;
    border-radius: 12px;
    background: var(--bg-primary);
    box-shadow: var(--shadow-container);
  }

  .main-layout {
    display: flex;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .global-modal-backdrop {
    position: absolute;
    inset: 0;
    z-index: 40;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 28px;
    background: rgba(0, 0, 0, 0.28);
    backdrop-filter: blur(10px);
  }

  .global-allowlist-modal {
    width: min(100%, 420px);
    border-radius: 24px;
    border: 1px solid var(--border-default);
    background: var(--bg-surface);
    box-shadow: var(--shadow-dropdown);
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .global-allowlist-modal-inner {
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .allowlist-modal-head,
  .allowlist-modal-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .allowlist-modal-head strong {
    font-size: 16px;
    color: var(--text-primary);
  }

  .modal-icon-button,
  .folder-picker-button,
  .modal-action {
    border: 0;
    cursor: pointer;
    transition:
      transform 0.14s ease,
      background 0.14s ease,
      opacity 0.14s ease;
  }

  .modal-icon-button:hover,
  .folder-picker-button:hover,
  .modal-action:hover {
    transform: translateY(-1px);
  }

  .modal-icon-button {
    width: 36px;
    height: 36px;
    border-radius: 12px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-hover);
    color: var(--text-secondary);
  }

  .folder-picker-button {
    min-height: 48px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    border-radius: 16px;
    background: var(--accent-primary);
    color: var(--text-on-dark);
    font: inherit;
    font-weight: 600;
  }

  .picked-folder {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 14px;
    border-radius: 18px;
    border: 1px dashed var(--border-input);
    background: var(--bg-surface);
  }

  .picked-folder.selected {
    border-style: solid;
    background: var(--bg-elevated);
  }

  .picked-folder-label {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-tertiary);
  }

  .picked-folder-path {
    font-size: 13px;
    color: var(--text-primary);
    word-break: break-word;
  }

  .allowlist-modal-feedback {
    margin: 0;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .allowlist-modal-feedback.error {
    color: var(--accent-danger);
  }

  .allowlist-name-input {
    width: 100%;
    box-sizing: border-box;
    border: 1px solid var(--border-input);
    border-radius: 14px;
    background: var(--bg-input);
    padding: 12px 14px;
    font: inherit;
    color: var(--text-primary);
  }

  .allowlist-modal-actions {
    justify-content: flex-end;
  }

  .modal-action {
    min-height: 38px;
    padding: 0 14px;
    border-radius: 12px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    font: inherit;
    font-size: 13px;
    font-weight: 600;
  }

  .modal-action.primary {
    background: var(--accent-primary);
    color: var(--text-on-dark);
  }

  .modal-action.secondary {
    background: var(--bg-hover);
    color: var(--text-secondary);
  }

  .modal-action:disabled {
    opacity: 0.45;
    transform: none;
    cursor: not-allowed;
  }

  .onboarding-layout {
    display: flex;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .center-area {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--bg-primary);
    overflow: hidden;
  }

  .loading-screen,
  .error-screen {
    width: 100vw;
    height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-primary);
  }

  .loading-content,
  .error-content {
    text-align: center;
  }

  .loading-content h2,
  .error-content h2 {
    margin-bottom: 8px;
    color: var(--text-primary);
  }

  .loading-content p,
  .error-content p {
    color: var(--text-muted);
    margin-bottom: 16px;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 10px 20px;
    border-radius: 10px;
    font-size: 14px;
    font-weight: 500;
    border: none;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn:hover {
    transform: translateY(-1px);
  }

  .btn-primary {
    background: var(--accent-primary);
    color: var(--text-on-dark);
  }

  .btn-primary:hover {
    opacity: 0.9;
  }
</style>
