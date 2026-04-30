<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { apiClient } from "../lib/api";
  import type { WorkingMemorySlot, NodeActivation } from "../lib/types";

  export let sessionId: string = "default";

  let wmSlots: WorkingMemorySlot[] = [];
  let topActivated: NodeActivation[] = [];
  let loading = false;
  let error = "";
  let refreshInterval: ReturnType<typeof setInterval> | null = null;

  async function loadBrainState() {
    loading = true;
    error = "";
    try {
      const [wmRes, activatedRes] = await Promise.all([
        apiClient.getBrainWorkingMemory(sessionId),
        apiClient.getBrainTopActivated(12)
      ]);
      wmSlots = wmRes.slots;
      topActivated = activatedRes.activations;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    loadBrainState();
    refreshInterval = setInterval(loadBrainState, 3000);
  });

  onDestroy(() => {
    if (refreshInterval) clearInterval(refreshInterval);
  });
</script>

<div class="brain-panel">
  <div class="brain-header">
    <h3>🧠 Brain Dashboard</h3>
    <span class="session-id">session: {sessionId}</span>
    {#if loading}
      <span class="loading">⟳</span>
    {/if}
  </div>

  {#if error}
    <div class="error">{error}</div>
  {/if}

  <div class="panels">
    <div class="panel">
      <h4>Working Memory ({wmSlots.length} slots)</h4>
      {#if wmSlots.length === 0}
        <div class="empty">No active WM slots</div>
      {:else}
        <div class="slot-list">
          {#each wmSlots as slot, i}
            <div class="slot-item" style="--relevance: {slot.relevance}">
              <div class="slot-header">
                <span class="slot-index">[{i + 1}]</span>
                <span class="slot-uri" title={slot.uri}>{slot.uri}</span>
                <span class="slot-score">{slot.relevance.toFixed(2)}</span>
              </div>
              <div class="slot-meta">
                <span class="source">{slot.source}</span>
                <span class="depth">{slot.injection_depth}</span>
                <span class="refreshes">×{slot.refresh_count}</span>
              </div>
              <div class="slot-content">
                {slot.content.slice(0, 200)}{slot.content.length > 200 ? "..." : ""}
              </div>
            </div>
          {/each}
        </div>
      {/if}
    </div>

    <div class="panel">
      <h4>Top Activated Nodes</h4>
      {#if topActivated.length === 0}
        <div class="empty">No activations yet</div>
      {:else}
        <div class="activation-list">
          {#each topActivated as act}
            <div class="activation-item" style="--activation: {act.current_activation}">
              <div class="activation-bar" style="width: {Math.min(act.current_activation * 100, 100)}%"></div>
              <div class="activation-info">
                <span class="node-id" title={act.node_id}>{act.node_id.slice(0, 8)}</span>
                <span class="current">{act.current_activation.toFixed(3)}</span>
                <span class="baseline">base: {act.baseline_activation.toFixed(3)}</span>
              </div>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  .brain-panel {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px;
    background: var(--surface-1, #1a1a1a);
    color: var(--text-1, #e0e0e0);
    border-radius: 8px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    font-size: 12px;
    max-height: 80vh;
    overflow-y: auto;
  }

  .brain-header {
    display: flex;
    align-items: center;
    gap: 12px;
    border-bottom: 1px solid var(--border, #333);
    padding-bottom: 8px;
  }

  .brain-header h3 {
    margin: 0;
    font-size: 14px;
  }

  .session-id {
    opacity: 0.6;
    font-size: 11px;
  }

  .loading {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .error {
    color: #ff6b6b;
    padding: 8px;
    background: rgba(255, 0, 0, 0.1);
    border-radius: 4px;
  }

  .panels {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  @media (max-width: 800px) {
    .panels {
      grid-template-columns: 1fr;
    }
  }

  .panel {
    background: var(--surface-2, #252525);
    border-radius: 6px;
    padding: 10px;
  }

  .panel h4 {
    margin: 0 0 8px 0;
    font-size: 12px;
    opacity: 0.8;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .empty {
    opacity: 0.4;
    padding: 16px;
    text-align: center;
  }

  .slot-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .slot-item {
    background: rgba(255, 255, 255, 0.03);
    border-left: 3px solid hsl(calc(var(--relevance) * 120), 70%, 50%);
    padding: 8px;
    border-radius: 0 4px 4px 0;
  }

  .slot-header {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 4px;
  }

  .slot-index {
    opacity: 0.5;
    min-width: 24px;
  }

  .slot-uri {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: #7ec8e3;
  }

  .slot-score {
    font-weight: bold;
    color: hsl(calc(var(--relevance) * 120), 70%, 60%);
  }

  .slot-meta {
    display: flex;
    gap: 8px;
    opacity: 0.5;
    font-size: 10px;
    margin-bottom: 4px;
  }

  .slot-content {
    opacity: 0.7;
    line-height: 1.4;
  }

  .activation-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .activation-item {
    position: relative;
    background: rgba(255, 255, 255, 0.03);
    padding: 6px 8px;
    border-radius: 4px;
    overflow: hidden;
  }

  .activation-bar {
    position: absolute;
    top: 0;
    left: 0;
    bottom: 0;
    background: hsla(calc(var(--activation) * 120), 70%, 50%, 0.15);
    transition: width 0.5s ease;
  }

  .activation-info {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    z-index: 1;
  }

  .node-id {
    opacity: 0.5;
    min-width: 60px;
  }

  .current {
    font-weight: bold;
    color: hsl(calc(var(--activation) * 120), 70%, 60%);
  }

  .baseline {
    opacity: 0.4;
    font-size: 10px;
  }
</style>
