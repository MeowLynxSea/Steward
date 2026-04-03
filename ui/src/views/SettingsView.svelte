<script lang="ts">
  import { X } from "lucide-svelte";
  import LlmConfigurationPanel from "../components/LlmConfigurationPanel.svelte";

  let { onClose }: { onClose: () => void } = $props();

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      onClose();
    }
  }

  function handleBackdropClick(event: MouseEvent) {
    if ((event.target as HTMLElement).classList.contains("settings-backdrop")) {
      onClose();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="settings-backdrop" onclick={handleBackdropClick}>
  <div class="settings-modal" role="dialog" aria-modal="true" aria-label="设置">
    <div class="settings-modal-header">
      <h3 class="settings-modal-title">设置</h3>
      <button class="settings-close-btn" onclick={onClose} aria-label="关闭设置">
        <X size={18} strokeWidth={2} />
      </button>
    </div>
    <div class="settings-modal-body">
      <LlmConfigurationPanel
        mode="settings"
        eyebrow="设置"
        title="模型服务"
        description="桌面运行时仅从持久化设置读取提供商配置。基于环境变量的模型路由已禁用。"
        submitLabel="保存更改"
      />
    </div>
  </div>
</div>

<style>
  .settings-backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: rgba(0, 0, 0, 0.35);
    backdrop-filter: blur(12px);
    animation: fadeIn 0.2s ease;
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .settings-modal {
    width: min(100%, 860px);
    max-height: calc(100vh - 48px);
    border-radius: 24px;
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
    box-shadow: var(--shadow-dropdown);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: modalIn 0.25s cubic-bezier(0.34, 1.56, 0.64, 1);
  }

  @keyframes modalIn {
    from {
      opacity: 0;
      transform: scale(0.92) translateY(12px);
    }
    to {
      opacity: 1;
      transform: scale(1) translateY(0);
    }
  }

  .settings-modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 18px 22px 0;
    flex-shrink: 0;
  }

  .settings-modal-title {
    font-size: 18px;
    font-weight: 700;
    color: var(--text-primary);
    margin: 0;
  }

  .settings-close-btn {
    width: 36px;
    height: 36px;
    border-radius: 10px;
    background: transparent;
    border: none;
    color: var(--text-tertiary);
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s ease, color 0.15s ease;
  }

  .settings-close-btn:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .settings-modal-body {
    flex: 1;
    overflow-y: auto;
    padding: 8px 22px 22px;
  }
</style>
