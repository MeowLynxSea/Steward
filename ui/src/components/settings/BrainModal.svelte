<script lang="ts">
  import { X, Brain } from "lucide-svelte";
  import BrainPanel from "../BrainPanel.svelte";

  let {
    sessionId = "default",
    onClose
  }: {
    sessionId?: string;
    onClose: () => void;
  } = $props();
</script>

<div class="modal-backdrop" onclick={onClose} onkeydown={(e) => e.key === 'Escape' && onClose()} role="dialog" aria-modal="true" tabindex="-1">
  <div class="modal-content" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()} role="presentation" tabindex="-1">
    <div class="modal-header">
      <div class="modal-title">
        <Brain size={18} />
        <span>Brain Dashboard</span>
      </div>
      <button class="close-btn" onclick={onClose} aria-label="Close">
        <X size={16} />
      </button>
    </div>
    <div class="modal-body">
      <BrainPanel {sessionId} />
    </div>
  </div>
</div>

<style>
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    padding: 20px;
  }

  .modal-content {
    background: var(--surface-1, #1a1a1a);
    border-radius: 12px;
    width: 90vw;
    max-width: 960px;
    max-height: 90vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.4);
  }

  .modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 18px;
    border-bottom: 1px solid var(--border, #333);
  }

  .modal-title {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary, #e0e0e0);
  }

  .close-btn {
    background: none;
    border: none;
    color: var(--text-secondary, #999);
    cursor: pointer;
    padding: 4px;
    border-radius: 4px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .close-btn:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--text-primary, #e0e0e0);
  }

  .modal-body {
    flex: 1;
    overflow: auto;
    padding: 12px;
  }
</style>
