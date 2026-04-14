<script lang="ts">
  import type { TaskRecord } from "../lib/types";

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
</script>

{#if modal}
  <div class="approval-modal-backdrop" role="presentation">
    <div
      class="approval-modal"
      role="dialog"
      aria-modal="true"
      aria-label="Approval required"
      tabindex="-1"
      bind:this={modalRef}
      onclick={stopPropagation}
      onkeydown={handleModalKeydown}
      onpointerdown={stopPropagation}
    >
      <div class="approval-modal-head">
        <div class="approval-title-wrap">
          <p class="eyebrow">Pending Approval</p>
          <h3>{pendingApproval?.risk ?? "Approval Required"}</h3>
        </div>
      </div>

      <div class="approval-modal-body">
        <div class="approval-summary-strip">
          <span class="approval-summary-label">Action</span>
          <p class="approval-subtitle">{pendingApproval?.summary}</p>
        </div>

        <div class="approval-operation-list" role="list">
          {#each pendingApproval?.operations ?? [] as operation, index}
            <div class="approval-operation-row" role="listitem">
              <div class="approval-operation-index">#{index + 1}</div>
              <div class="approval-operation-main">
                <div class="approval-operation-topline">
                  <strong>{operation.kind}</strong>
                  <span>{operation.tool_name}</span>
                </div>
                <div class="approval-operation-meta">
                  <span>{operation.path ?? "Unknown source"}</span>
                  <span>{operation.destination_path ?? "No destination"}</span>
                </div>
              </div>
            </div>
          {/each}
        </div>

        {#if showForm}
          <label class="field">
            <span>Reject reason</span>
            <textarea
              bind:value={rejectReason}
              rows="3"
              placeholder="Explain why this run should stop"
            ></textarea>
          </label>
        {/if}

        <div class="action-row approval-actions">
          <button class="button button-primary" type="button" onclick={onApprove}>Approve</button>
          {#if pendingApproval?.allow_always && onApproveAlways}
            <button class="button button-secondary" type="button" onclick={onApproveAlways}>
              Always Allow
            </button>
          {/if}
          <button class="button button-ghost" type="button" onclick={onReject}>Reject</button>
        </div>
      </div>
    </div>
  </div>
{:else}
  <article class="feature-card soft-card">
    <div class="card-head">
      <div>
        <p class="eyebrow">Pending Approval</p>
        <h3>{pendingApproval?.risk ?? "Approval"}</h3>
      </div>
    </div>

    <p class="muted">{pendingApproval?.summary}</p>

    <div class="stack compact">
      {#each pendingApproval?.operations ?? [] as operation, index}
        <article class="mini-card">
          <div class="mini-card-head">
            <strong>#{index + 1} {operation.kind}</strong>
            <span>{operation.tool_name}</span>
          </div>
          <span>{operation.path ?? "Unknown source"}</span>
          <span>{operation.destination_path ?? "No destination"}</span>
        </article>
      {/each}
    </div>

    {#if showForm}
      <label class="field">
        <span>Reject reason</span>
        <textarea bind:value={rejectReason} rows="3" placeholder="Explain why this run should stop"></textarea>
      </label>
    {/if}

    <div class="action-row">
      <button class="button button-primary" type="button" onclick={onApprove}>Approve</button>
      {#if pendingApproval?.allow_always && onApproveAlways}
        <button class="button button-secondary" type="button" onclick={onApproveAlways}>Always Allow</button>
      {/if}
      <button class="button button-ghost" type="button" onclick={onReject}>Reject</button>
    </div>
  </article>
{/if}

<style>
  .approval-modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 70;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: rgba(15, 23, 42, 0.32);
    backdrop-filter: blur(10px);
    animation: approval-backdrop-in 0.18s ease-out both;
  }

  .approval-modal {
    width: min(560px, 100%);
    max-height: min(80vh, 760px);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid color-mix(in srgb, var(--accent-gold) 18%, var(--border-default));
    border-radius: 18px;
    background: linear-gradient(
      180deg,
      color-mix(in srgb, var(--bg-sidebar) 82%, var(--accent-gold) 18%) 0%,
      var(--bg-surface) 100%
    );
    box-shadow: 0 24px 80px rgba(15, 23, 42, 0.18);
    outline: none;
    animation: approval-modal-in 0.22s cubic-bezier(0.22, 1, 0.36, 1) both;
  }

  .approval-modal-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 18px 20px 14px;
    border-bottom: 1px solid color-mix(in srgb, var(--accent-gold) 14%, var(--border-default));
    background: transparent;
  }

  .approval-title-wrap {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }

  .approval-title-wrap h3 {
    margin: 0;
    font-size: 22px;
    line-height: 1.2;
    color: var(--text-primary);
  }

  .approval-subtitle {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
    color: var(--text-secondary);
  }

  .approval-modal-body {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 16px 20px 20px;
    overflow: auto;
    background: transparent;
  }

  .approval-summary-strip {
    display: grid;
    gap: 6px;
    padding: 12px 14px;
    border-radius: 12px;
    background: color-mix(in srgb, var(--accent-gold) 12%, var(--bg-surface));
    border: 1px solid color-mix(in srgb, var(--accent-gold) 20%, transparent);
  }

  .approval-summary-label {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--accent-gold) 72%, var(--text-secondary));
  }

  .approval-operation-list {
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border-default);
    border-bottom: 1px solid var(--border-default);
  }

  .approval-operation-row {
    display: grid;
    grid-template-columns: 40px minmax(0, 1fr);
    gap: 12px;
    padding: 14px 0;
    align-items: start;
    border-bottom: 1px solid var(--border-subtle);
  }

  .approval-operation-row:last-child {
    border-bottom: none;
  }

  .approval-operation-index {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-height: 28px;
    padding: 0 8px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent-gold) 16%, var(--bg-surface));
    color: color-mix(in srgb, var(--accent-gold) 76%, var(--text-primary));
    font-size: 12px;
    font-weight: 700;
  }

  .approval-operation-main {
    display: grid;
    gap: 6px;
    min-width: 0;
  }

  .approval-operation-topline {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    min-width: 0;
  }

  .approval-operation-topline strong {
    color: var(--text-primary);
    text-transform: capitalize;
  }

  .approval-operation-topline span {
    font-size: 13px;
    color: color-mix(in srgb, var(--accent-gold) 64%, var(--text-secondary));
    text-align: right;
  }

  .approval-operation-meta {
    display: grid;
    gap: 4px;
    font-size: 13px;
    color: var(--text-secondary);
    word-break: break-word;
  }

  .approval-actions {
    justify-content: flex-end;
  }

  @keyframes approval-backdrop-in {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  @keyframes approval-modal-in {
    from {
      opacity: 0;
      transform: translateY(10px) scale(0.985);
    }
    to {
      opacity: 1;
      transform: translateY(0) scale(1);
    }
  }

  @media (max-width: 720px) {
    .approval-modal-backdrop {
      padding: 16px;
      align-items: flex-end;
    }

    .approval-modal {
      width: 100%;
      max-height: min(84vh, 760px);
      border-radius: 18px 18px 0 0;
    }

    .approval-modal-head,
    .approval-modal-body {
      padding-left: 16px;
      padding-right: 16px;
    }

    .approval-operation-row {
      grid-template-columns: 1fr;
      gap: 10px;
    }

    .approval-operation-topline {
      flex-direction: column;
      align-items: flex-start;
    }

    .approval-operation-topline span {
      text-align: left;
    }
  }
</style>
