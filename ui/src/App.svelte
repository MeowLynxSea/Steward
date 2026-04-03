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
  let showMountModal = $state(false);
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
        : "Failed to connect to IronCowork backend. Is the server running?";
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

{#if appLoading}
  <div class="loading-screen">
    <div class="loading-content">
      <h2>加载中...</h2>
      <p>正在连接到 IronCowork 后端</p>
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
      title="IronCowork"
      leftSidebarCollapsed={true}
      rightSidebarCollapsed={true}
      onToggleLeft={() => undefined}
      onToggleRight={() => undefined}
    />
    <div class="onboarding-layout">
      <OnboardingView onComplete={handleOnboardingComplete} />
    </div>
  </div>
{:else if router.current === "settings"}
  <SettingsView />
{:else}
  <div class="app-container">
    <TitleBar
      title="IronCowork"
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
        onSettings={() => router.navigate("settings")}
      />

      <div class="center-area">
        <ChatArea
          session={sessionsStore.active}
          task={sessionsStore.active?.current_task ?? null}
          modelName={settingsStore.data.selected_model}
          loading={sessionsStore.loading}
          onSendMessage={handleSendMessage}
          onApproveTask={handleApproveTask}
          onApproveTaskAlways={handleApproveTaskAlways}
          onRejectTask={handleRejectTask}
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
    color: #3d3d3d;
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
    border-radius: 18px;
    background: #f5f0e8;
    box-shadow: 0 10px 30px rgba(33, 28, 24, 0.12);
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
    background: rgba(28, 20, 10, 0.28);
    backdrop-filter: blur(10px);
  }

  .global-mount-modal {
    width: min(100%, 420px);
    border-radius: 24px;
    border: 1px solid rgba(92, 72, 40, 0.14);
    background:
      radial-gradient(circle at top right, rgba(214, 184, 108, 0.18), transparent 34%),
      linear-gradient(180deg, #fffaf0 0%, #f6efdf 100%);
    box-shadow: 0 24px 60px rgba(47, 32, 12, 0.22);
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
    color: #3e301b;
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
    background: rgba(122, 94, 52, 0.08);
    color: #5c4828;
  }

  .folder-picker-button {
    min-height: 48px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    border-radius: 16px;
    background: #5c4828;
    color: #fff9ee;
    font: inherit;
    font-weight: 600;
  }

  .picked-folder {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 14px;
    border-radius: 18px;
    border: 1px dashed rgba(92, 72, 40, 0.18);
    background: rgba(255, 255, 255, 0.62);
  }

  .picked-folder.selected {
    border-style: solid;
    background: rgba(201, 150, 57, 0.1);
  }

  .picked-folder-label {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: #7b6441;
  }

  .picked-folder-path {
    font-size: 13px;
    color: #3f301d;
    word-break: break-word;
  }

  .mount-name-input {
    width: 100%;
    box-sizing: border-box;
    border: 1px solid rgba(92, 72, 40, 0.14);
    border-radius: 14px;
    background: rgba(255, 255, 255, 0.88);
    padding: 12px 14px;
    font: inherit;
    color: #3b2b18;
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
    background: #5c4828;
    color: #fff9ee;
  }

  .modal-action.secondary {
    background: rgba(122, 94, 52, 0.11);
    color: #5c4828;
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
    background: #f5f0e8;
    overflow: hidden;
  }

  .loading-screen,
  .error-screen {
    width: 100vw;
    height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #f5f0e8;
  }

  .loading-content,
  .error-content {
    text-align: center;
  }

  .loading-content h2,
  .error-content h2 {
    margin-bottom: 8px;
    color: #3d3d3d;
  }

  .loading-content p,
  .error-content p {
    color: rgba(61, 61, 61, 0.6);
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
    background: #3d3d3d;
    color: #ffffff;
  }

  .btn-primary:hover {
    background: #2a2a2a;
  }
</style>
