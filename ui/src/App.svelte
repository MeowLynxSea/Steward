<script lang="ts">
  import { onMount } from "svelte";
  import ChatArea from "./components/ChatArea.svelte";
  import LeftSidebar from "./components/LeftSidebar.svelte";
  import RightSidebar from "./components/RightSidebar.svelte";
  import TitleBar from "./components/TitleBar.svelte";
  import { router } from "./lib/router.svelte";
  import { settingsStore } from "./lib/stores/settings.svelte";
  import { sessionsStore } from "./lib/stores/sessions.svelte";
  import { tasksStore } from "./lib/stores/tasks.svelte";
  import { workspaceStore } from "./lib/stores/workspace.svelte";
  import { workbenchStore } from "./lib/stores/workbench.svelte";
  import { listenForFolderDrops } from "./lib/tauri";
  import type { TaskRecord, WorkspaceSearchResult } from "./lib/types";
  import OnboardingView from "./views/OnboardingView.svelte";
  import SettingsView from "./views/SettingsView.svelte";

  let appLoading = $state(true);
  let appError = $state("");
  let leftSidebarCollapsed = $state(false);
  let rightSidebarCollapsed = $state(false);

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

  function handleRejectTask(task: TaskRecord, reason: string) {
    void tasksStore.reject(task, reason);
  }

  function handleUseWorkspaceResult(result: WorkspaceSearchResult) {
    const snippet = `Use workspace context from ${result.document_path}:\n${result.content}`;
    console.log("Using workspace result:", snippet);
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
          onRejectTask={handleRejectTask}
        />
      </div>

      <RightSidebar
        entries={workspaceStore.entries}
        searchResults={workspaceStore.searchResults}
        searchQuery={workspaceStore.searchQuery}
        collapsed={rightSidebarCollapsed}
        onSearch={(query) => void workspaceStore.search(query)}
        onRefresh={() => void workspaceStore.refresh()}
        onUseResult={handleUseWorkspaceResult}
      />
    </div>
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
