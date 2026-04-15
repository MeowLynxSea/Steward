<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import { ShieldAlert } from "lucide-svelte";
  import type { TaskRecord, TaskOperation } from "../lib/types";

  let {
    task,
    rejectReason = $bindable(),
    showForm = true,
    modal = false,
    onApprove,
    onApproveAlways,
    onReject
  }: {
    task: TaskRecord;
    rejectReason: string;
    showForm?: boolean;
    modal?: boolean;
    onApprove: () => void;
    onApproveAlways?: () => void;
    onReject: () => void;
  } = $props();

  let modalRef: HTMLDivElement | null = $state(null);
  let rejectStep = $state(false);

  const pendingApproval = $derived(task.pending_approval);

  $effect(() => {
    if (modal) {
      modalRef?.focus();
    }
  });

  function stopPropagation(event: Event) {
    event.stopPropagation();
  }

  function handleModalKeydown(event: KeyboardEvent) {
    event.stopPropagation();
  }

  function handleRejectClick() {
    if (showForm) {
      rejectStep = true;
    } else {
      onReject();
    }
  }

  function confirmReject() {
    onReject();
    rejectStep = false;
  }

  function cancelReject() {
    rejectStep = false;
    rejectReason = "";
  }

  function formatParamValue(val: unknown): string {
    if (val === null || val === undefined) return "";
    if (typeof val === "string") {
      return val.length > 120 ? val.slice(0, 117) + "…" : val;
    }
    const s = JSON.stringify(val);
    return s.length > 120 ? s.slice(0, 117) + "…" : s;
  }

  function getDisplayParams(op: TaskOperation): Array<[string, string]> {
    if (!op.parameters) return [];
    return Object.entries(op.parameters)
      .filter(([_, v]) => v !== null && v !== undefined && v !== "")
      .slice(0, 5)
      .map(([k, v]) => [k, formatParamValue(v)]);
  }
</script>

{#snippet operationCard(op: TaskOperation, index: number)}
  <div class="op-card">
    <div class="op-head">
      <span class="op-index">#{index + 1}</span>
      <strong class="op-tool">{op.tool_name}</strong>
    </div>
    {#if op.path}
      <div class="op-path">{op.path}{#if op.destination_path} → {op.destination_path}{/if}</div>
    {/if}
    {#each getDisplayParams(op) as [key, val]}
      <div class="op-param">
        <span class="param-key">{key}</span>
        <span class="param-val">{val}</span>
      </div>
    {/each}
  </div>
{/snippet}

{#if modal}
  <div
    class="approval-overlay"
    in:fade={{ duration: 200 }}
    out:fade={{ duration: 180 }}
    role="presentation"
  >
    <div class="approval-backdrop"></div>

    {#if !rejectStep}
      <div
        class="approval-sheet"
        role="dialog"
        aria-modal="true"
        aria-label="需要审批"
        tabindex="-1"
        bind:this={modalRef}
        onclick={stopPropagation}
        onkeydown={handleModalKeydown}
        onpointerdown={stopPropagation}
        in:fly={{ y: 100, duration: 320, easing: (t) => 1 - Math.pow(1 - t, 3) }}
        out:fly={{ y: 100, duration: 240, easing: (t) => t * t }}
      >
        <div class="sheet-header">
          <ShieldAlert size={16} strokeWidth={2} />
          <span class="sheet-title">助手想要执行以下操作</span>
        </div>

        <div class="sheet-ops">
          {#each pendingApproval?.operations ?? [] as op, i}
            {@render operationCard(op, i)}
          {/each}
        </div>

        <div class="sheet-actions">
          <button class="btn btn-approve" type="button" onclick={onApprove}>允许</button>
          {#if pendingApproval?.allow_always && onApproveAlways}
            <button class="btn btn-always" type="button" onclick={onApproveAlways}>始终允许</button>
          {/if}
          <button class="btn btn-reject" type="button" onclick={handleRejectClick}>拒绝</button>
        </div>
      </div>
    {:else}
      <div
        class="approval-sheet reject-sheet"
        role="dialog"
        aria-modal="true"
        aria-label="拒绝原因"
        tabindex="-1"
        onclick={stopPropagation}
        onkeydown={handleModalKeydown}
        onpointerdown={stopPropagation}
        in:fly={{ y: 100, duration: 320, easing: (t) => 1 - Math.pow(1 - t, 3) }}
        out:fly={{ y: 100, duration: 240, easing: (t) => t * t }}
      >
        <div class="sheet-header">
          <ShieldAlert size={16} strokeWidth={2} />
          <span class="sheet-title">告诉助手为什么拒绝</span>
        </div>

        <textarea
          class="reject-input"
          bind:value={rejectReason}
          rows="3"
          placeholder="简要说明原因，帮助助手理解（可选）"
        ></textarea>

        <div class="sheet-actions">
          <button class="btn btn-reject-confirm" type="button" onclick={confirmReject}>确认拒绝</button>
          <button class="btn btn-cancel" type="button" onclick={cancelReject}>取消</button>
        </div>
      </div>
    {/if}
  </div>
{:else}
  <article class="approval-inline">
    <div class="sheet-header">
      <ShieldAlert size={16} strokeWidth={2} />
      <span class="sheet-title">助手想要执行以下操作</span>
    </div>

    <div class="sheet-ops">
      {#each pendingApproval?.operations ?? [] as op, i}
        {@render operationCard(op, i)}
      {/each}
    </div>

    {#if !rejectStep}
      <div class="sheet-actions">
        <button class="btn btn-approve" type="button" onclick={onApprove}>允许</button>
        {#if pendingApproval?.allow_always && onApproveAlways}
          <button class="btn btn-always" type="button" onclick={onApproveAlways}>始终允许</button>
        {/if}
        <button class="btn btn-reject" type="button" onclick={handleRejectClick}>拒绝</button>
      </div>
    {:else}
      <textarea
        class="reject-input"
        bind:value={rejectReason}
        rows="3"
        placeholder="简要说明原因，帮助助手理解（可选）"
      ></textarea>
      <div class="sheet-actions">
        <button class="btn btn-reject-confirm" type="button" onclick={confirmReject}>确认拒绝</button>
        <button class="btn btn-cancel" type="button" onclick={cancelReject}>取消</button>
      </div>
    {/if}
  </article>
{/if}

<style>
  /* ── Full-area overlay (modal mode) ── */
  .approval-overlay {
    position: absolute;
    inset: 0;
    z-index: 60;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    align-items: center;
    padding: 0 16px 16px;
    pointer-events: auto;
  }

  .approval-backdrop {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.1);
    backdrop-filter: blur(6px);
    animation: backdrop-blur-in 0.3s ease-out both;
  }

  @keyframes backdrop-blur-in {
    from {
      backdrop-filter: blur(0);
      background: rgba(0, 0, 0, 0);
    }
    to {
      backdrop-filter: blur(6px);
      background: rgba(0, 0, 0, 0.1);
    }
  }

  .approval-sheet {
    position: relative;
    z-index: 1;
    width: min(520px, 100%);
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px;
    border-radius: 16px;
    background: var(--bg-sidebar);
    border: 1px solid color-mix(in srgb, var(--accent-gold) 18%, var(--border-default));
    box-shadow:
      0 -4px 24px rgba(0, 0, 0, 0.08),
      0 0 0 1px rgba(0, 0, 0, 0.03);
    backdrop-filter: blur(16px);
  }

  /* ── Sheet header ── */
  .sheet-header {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--accent-gold);
  }

  .sheet-title {
    font-size: 13px;
    font-weight: 650;
    color: var(--text-primary);
  }

  /* ── Operation cards ── */
  .sheet-ops {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 240px;
    overflow-y: auto;
  }

  .op-card {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
  }

  .op-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .op-index {
    font-size: 11px;
    font-weight: 700;
    color: var(--accent-gold);
    flex-shrink: 0;
  }

  .op-tool {
    font-size: 13px;
    font-weight: 650;
    color: var(--text-primary);
    font-family: ui-monospace, "SF Mono", "Cascadia Code", Menlo, monospace;
  }

  .op-path {
    font-size: 12px;
    color: var(--text-tertiary);
    word-break: break-all;
    padding-left: 20px;
  }

  .op-param {
    display: flex;
    gap: 8px;
    padding-left: 20px;
    font-size: 12px;
    line-height: 1.4;
  }

  .param-key {
    color: var(--text-tertiary);
    flex-shrink: 0;
    font-family: ui-monospace, "SF Mono", "Cascadia Code", Menlo, monospace;
  }

  .param-key::after {
    content: ":";
  }

  .param-val {
    color: var(--text-secondary);
    word-break: break-all;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  /* ── Action buttons ── */
  .sheet-actions {
    display: flex;
    gap: 8px;
  }

  .btn {
    height: 36px;
    padding: 0 14px;
    border: 1px solid transparent;
    border-radius: 10px;
    font-size: 13px;
    font-weight: 600;
    font-family: inherit;
    cursor: pointer;
    transition: background 0.15s, border-color 0.15s, opacity 0.15s;
  }

  .btn-approve {
    background: var(--accent-gold);
    color: #fff;
    flex: 1;
  }

  .btn-approve:hover {
    opacity: 0.88;
  }

  .btn-always {
    background: color-mix(in srgb, var(--accent-gold) 14%, var(--bg-surface));
    color: var(--accent-gold);
    border-color: color-mix(in srgb, var(--accent-gold) 24%, transparent);
  }

  .btn-always:hover {
    background: color-mix(in srgb, var(--accent-gold) 22%, var(--bg-surface));
  }

  .btn-reject {
    background: var(--bg-input);
    color: var(--text-secondary);
    border-color: var(--border-input);
  }

  .btn-reject:hover {
    background: var(--bg-elevated);
    color: var(--accent-danger-text);
    border-color: color-mix(in srgb, var(--accent-danger-text) 24%, transparent);
  }

  .btn-reject-confirm {
    background: var(--accent-danger-text);
    color: #fff;
    flex: 1;
  }

  .btn-reject-confirm:hover {
    opacity: 0.88;
  }

  .btn-cancel {
    background: var(--bg-input);
    color: var(--text-secondary);
    border-color: var(--border-input);
  }

  .btn-cancel:hover {
    background: var(--bg-elevated);
  }

  /* ── Reject input ── */
  .reject-input {
    width: 100%;
    min-height: 64px;
    padding: 10px 12px;
    border: 1px solid var(--border-input);
    border-radius: 10px;
    background: var(--bg-input);
    color: var(--text-primary);
    font-size: 13px;
    font-family: inherit;
    line-height: 1.5;
    resize: vertical;
    transition: border-color 0.15s, box-shadow 0.15s;
  }

  .reject-input:hover {
    border-color: color-mix(in srgb, var(--border-input) 60%, var(--text-tertiary));
  }

  .reject-input:focus {
    outline: none;
    border-color: var(--accent-gold);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-gold) 14%, transparent);
  }

  /* ── Inline (non-modal) card ── */
  .approval-inline {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 14px;
    border-radius: 14px;
    border: 1px solid color-mix(in srgb, var(--accent-gold) 18%, var(--border-default));
    background: color-mix(in srgb, var(--accent-gold) 4%, var(--bg-surface));
  }

  /* ── Responsive ── */
  @media (max-width: 720px) {
    .approval-overlay {
      padding: 0 12px 12px;
    }
  }
</style>
