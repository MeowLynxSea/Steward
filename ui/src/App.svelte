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
  import { workbenchStore } from "./lib/stores/workbench.svelte";
  import { listenForFolderDrops, pickDirectory } from "./lib/tauri";
  import type { TaskRecord, WorkspaceSearchResult } from "./lib/types";
  import OnboardingView from "./views/OnboardingView.svelte";
  import SettingsView from "./views/SettingsView.svelte";

  let appLoading = $state(true);
  let appError = $state("");
  let leftSidebarCollapsed = $state(false);
  let rightSidebarCollapsed = $state(false);
  let showSettings = $state(false);
  let showMountModal = $state(false);

  function openSettings() {
    showSettings = true;
  }

  function closeSettings() {
    showSettings = false;
    router.navigate("sessions");
  }

  // Sync settings modal state with router
  $effect(() => {
    if (router.current === "settings") {
      showSettings = true;
    }
  });
  let mountDisplayName = $state("");
  let selectedMountPath = $state("");
  let selectingMountPath = $state(false);

  async function loadWorkspaceData() {
    await Promise.all([
      sessionsStore.fetchList(),
      tasksStore.fetch(),
      workspaceStore.fetch(),
      workbenchStore.fetch()
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

  function handleSendMessage(content: string) {
    void sessionsStore.sendMessage(content);
  }

  function handleSuggestionClick(suggestion: string) {
    void sessionsStore.sendMessage(suggestion);
  }

  function handleSelectModel(model: string) {
    settingsStore.updateField("selected_model", model);
    void settingsStore.save();
  }

  const availableModels = $derived.by(() => {
    const models: string[] = [];
    const current = settingsStore.data.selected_model;
    if (current) models.push(current);

    for (const provider of settingsStore.data.llm_custom_providers) {
      if (provider.default_model && !models.includes(provider.default_model)) {
        models.push(provider.default_model);
      }
    }
    return models;
  });

  function handleApproveTask(task: TaskRecord) {
    void tasksStore.approve(task);
  }

  function handleApproveTaskAlways(task: TaskRecord) {
    void tasksStore.approve(task, true);
  }

  function handleRejectTask(task: TaskRecord, reason: string) {
    void tasksStore.reject(task, reason);
  }

  function handleUseWorkspaceResult(result: WorkspaceSearchResult) {
    const snippet = `Use workspace context from ${result.document_path}:\n${result.content}`;
    console.log("Using workspace result:", snippet);
  }

  function openMountModal() {
    showMountModal = true;
  }

  function closeMountModal() {
    showMountModal = false;
    mountDisplayName = "";
    selectedMountPath = "";
    selectingMountPath = false;
  }

  async function handlePickMountDirectory() {
    selectingMountPath = true;
    try {
      const selected = await pickDirectory();
      if (selected) {
        selectedMountPath = selected;
      }
    } finally {
      selectingMountPath = false;
    }
  }

  async function handleCreateMount() {
    if (!selectedMountPath) return;
    await workspaceStore.createMount(selectedMountPath, mountDisplayName.trim() || undefined);
    if (!workspaceStore.error) {
      closeMountModal();
    }
  }

  onMount(async () => {
    themeStore.init();
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
          task={sessionsStore.active?.active_thread_task ?? null}
          streaming={sessionsStore.streaming}
          modelName={settingsStore.data.selected_model}
          availableModels={availableModels}
          loading={sessionsStore.loading}
          onSendMessage={handleSendMessage}
          onSuggestionClick={handleSuggestionClick}
          onApproveTask={handleApproveTask}
          onApproveTaskAlways={handleApproveTaskAlways}
          onRejectTask={handleRejectTask}
          onSelectModel={handleSelectModel}
        />
      </div>

      <RightSidebar
        currentPath={workspaceStore.path}
        entries={workspaceStore.entries}
        searchResults={workspaceStore.searchResults}
        searchQuery={workspaceStore.searchQuery}
        selectedMount={workspaceStore.selectedMount}
        mountDiff={workspaceStore.mountDiff}
        collapsed={rightSidebarCollapsed}
        onSearch={(query) => void workspaceStore.search(query)}
        onRefresh={() => void workspaceStore.refresh()}
        onNavigate={(path) => void workspaceStore.fetch(path)}
        onRequestMount={openMountModal}
        onOpenEntry={(entry) => {
          if (entry.kind === "mount" && entry.path) {
            void workspaceStore.loadMount(entry.path);
            void workspaceStore.fetch(entry.uri ?? "workspace://mounts");
          } else if (entry.is_directory && entry.uri) {
            void workspaceStore.fetch(entry.uri);
          }
        }}
        onKeepMount={(mountId, scopePath, checkpointId) => void workspaceStore.keepMount(mountId, scopePath, checkpointId)}
        onRevertMount={(mountId, scopePath, checkpointId) => void workspaceStore.revertMount(mountId, scopePath, checkpointId)}
        onCreateCheckpoint={(mountId, label, summary) => void workspaceStore.createCheckpoint(mountId, label, summary)}
        onResolveConflict={(mountId, path, resolution, renamedCopyPath, mergedContent) =>
          void workspaceStore.resolveConflict(mountId, path, resolution, renamedCopyPath, mergedContent)}
        onUseResult={handleUseWorkspaceResult}
      />
    </div>

    {#if showMountModal}
      <div
        class="global-modal-backdrop"
        role="presentation"
        onclick={closeMountModal}
        transition:fade={{ duration: 180 }}
        onkeydown={(event) => event.key === "Escape" && closeMountModal()}
      >
        <div
          class="global-mount-modal"
          role="dialog"
          aria-modal="true"
          aria-label="挂载目录"
          tabindex="-1"
          in:scale={{ duration: 220, start: 0.92 }}
          out:scale={{ duration: 150, start: 0.96 }}
        >
          <div class="global-mount-modal-inner" role="presentation" onclick={(event) => event.stopPropagation()}>
            <div class="mount-modal-head">
              <strong>挂载目录</strong>
              <button class="modal-icon-button" onclick={closeMountModal} aria-label="关闭">
                <X size={18} strokeWidth={2} />
              </button>
            </div>

            <button class="folder-picker-button" onclick={() => void handlePickMountDirectory()}>
              <FolderSearch size={18} strokeWidth={2} />
              {selectingMountPath ? "正在打开选择器..." : selectedMountPath ? "重新选择文件夹" : "选择文件夹"}
            </button>

            <div class="picked-folder {selectedMountPath ? 'selected' : ''}">
              <span class="picked-folder-label">已选目录</span>
              <span class="picked-folder-path">{selectedMountPath || "尚未选择文件夹"}</span>
            </div>

            <input
              class="mount-name-input"
              type="text"
              bind:value={mountDisplayName}
              placeholder="显示名称（可选）"
              onkeydown={(event) => event.key === "Enter" && void handleCreateMount()}
            />

            <div class="mount-modal-actions">
              <button class="modal-action secondary" onclick={closeMountModal}>取消</button>
              <button
                class="modal-action primary"
                onclick={() => void handleCreateMount()}
                disabled={!selectedMountPath || workspaceStore.loading}
              >
                <FolderPlus size={16} strokeWidth={2} />
                创建挂载
              </button>
            </div>
          </div>
        </div>
      </div>
    {/if}

    {#if showSettings}
      <SettingsView onClose={closeSettings} />
    {/if}
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

  .global-mount-modal {
    width: min(100%, 420px);
    border-radius: 24px;
    border: 1px solid var(--border-default);
    background: var(--bg-surface);
    box-shadow: var(--shadow-dropdown);
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .global-mount-modal-inner {
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .mount-modal-head,
  .mount-modal-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .mount-modal-head strong {
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

  .mount-name-input {
    width: 100%;
    box-sizing: border-box;
    border: 1px solid var(--border-input);
    border-radius: 14px;
    background: var(--bg-input);
    padding: 12px 14px;
    font: inherit;
    color: var(--text-primary);
  }

  .mount-modal-actions {
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
