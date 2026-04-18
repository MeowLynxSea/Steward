<script lang="ts">
  import { fade, fly } from "svelte/transition";
  import StatusBadge from "../components/StatusBadge.svelte";
  import TaskApprovalCard from "../components/TaskApprovalCard.svelte";
  import { formatDateTime, timelineTitle } from "../lib/presentation";
  import { tasksStore } from "../lib/stores/tasks.svelte";
  import type { TaskRecord } from "../lib/types";

  let rejectReason = $state("Rejected by user");
  let yoloCandidate = $state<TaskRecord | null>(null);

  function handleToggleMode(task: TaskRecord) {
    if (task.mode === "yolo") {
      void tasksStore.toggleMode(task);
      return;
    }

    yoloCandidate = task;
  }

  function closeYoloModal() {
    yoloCandidate = null;
  }

  function confirmYoloMode() {
    if (!yoloCandidate) {
      return;
    }
    void tasksStore.toggleMode(yoloCandidate);
    yoloCandidate = null;
  }
</script>

<section class="view-grid split-grid">
  <section class="panel column-panel">
    <div class="card-head">
      <div>
        <p class="eyebrow">Thread State</p>
        <h2>Execution snapshots</h2>
      </div>
      <button class="button button-primary" onclick={() => void tasksStore.refresh()}>Refresh</button>
    </div>

    {#if tasksStore.pendingApprovals.length > 0}
      <div class="stack compact">
        <span class="section-label">Approval Center</span>
        {#each tasksStore.pendingApprovals as task}
          <button
            class={`session-tile warning ${task.id === tasksStore.activeId ? "active" : ""}`}
            onclick={() => void tasksStore.select(task.id)}
          >
            <strong>{task.title}</strong>
            <span>{task.pending_approval?.risk ?? "approval"} · {task.mode}</span>
          </button>
        {/each}
      </div>
    {/if}

    {#if tasksStore.loading}
      <p class="muted">Loading execution snapshots...</p>
    {:else if tasksStore.list.length === 0}
      <p class="muted">No execution snapshots yet. Start from the session workbench.</p>
    {:else}
      <div class="stack">
        {#each tasksStore.list as task}
          <button
            class={`session-tile ${task.id === tasksStore.activeId ? "active" : ""}`}
            onclick={() => void tasksStore.select(task.id)}
          >
            <strong>{task.title}</strong>
            <span>{task.status} · {task.mode}</span>
            <span>{formatDateTime(task.updated_at)}</span>
          </button>
        {/each}
      </div>
    {/if}
  </section>

  <section class="panel detail-panel">
    {#if tasksStore.detailLoading}
      <p class="muted">Loading run detail...</p>
    {:else if tasksStore.detail}
      <div class="hero-panel compact">
        <div>
          <p class="eyebrow">Thread Execution Detail</p>
          <h2>{tasksStore.detail.task.title}</h2>
          <p class="muted">{tasksStore.detail.task.id}</p>
        </div>

        <div class="hero-status">
          <StatusBadge status={tasksStore.detail.task.status} />
          <p>{tasksStore.detail.task.current_step?.title ?? "Thread execution state updated"}</p>
        </div>
      </div>

      <div class="toolbar">
        <button class="button button-ghost" onclick={() => handleToggleMode(tasksStore.detail!.task)}>
          {tasksStore.detail.task.mode === "yolo" ? "Switch To Ask" : "Switch To Yolo"}
        </button>
        {#if !["completed", "failed", "rejected", "cancelled"].includes(tasksStore.detail.task.status)}
          <button class="button button-ghost" onclick={() => void tasksStore.cancel(tasksStore.detail!.task)}>Cancel</button>
        {/if}
        {#if tasksStore.detail.task.status === "waiting_approval" && tasksStore.detail.task.pending_approval}
          <button class="button button-primary" onclick={() => void tasksStore.approve(tasksStore.detail!.task)}>Approve</button>
          {#if tasksStore.detail.task.pending_approval.allow_always}
            <button class="button button-secondary" onclick={() => void tasksStore.approve(tasksStore.detail!.task, true)}>
              Always Allow
            </button>
          {/if}
          <button class="button button-secondary" onclick={() => void tasksStore.reject(tasksStore.detail!.task, rejectReason)}>Reject</button>
        {/if}
      </div>

      <div class="metric-grid">
        <article class="mini-card">
          <span class="section-label">Mode</span>
          <strong>{tasksStore.detail.task.mode}</strong>
        </article>
        <article class="mini-card">
          <span class="section-label">Updated</span>
          <strong>{formatDateTime(tasksStore.detail.task.updated_at)}</strong>
        </article>
        <article class="mini-card">
          <span class="section-label">Timeline Events</span>
          <strong>{tasksStore.detail.timeline.length}</strong>
        </article>
      </div>

      {#if tasksStore.detail.task.pending_approval}
        <TaskApprovalCard
          task={tasksStore.detail.task}
          bind:rejectReason
          onApprove={() => void tasksStore.approve(tasksStore.detail!.task)}
          onApproveAlways={() => void tasksStore.approve(tasksStore.detail!.task, true)}
          onReject={() => void tasksStore.reject(tasksStore.detail!.task, rejectReason)}
        />
      {/if}

      {#if tasksStore.detail.task.status === "rejected" || tasksStore.detail.task.status === "failed"}
        <article class="feature-card soft-card">
          <div class="card-head">
            <div>
              <p class="eyebrow">Issue</p>
              <h3>{tasksStore.detail.task.status === "rejected" ? "Rejection Reason" : "Failure Reason"}</h3>
            </div>
          </div>
          <p>{tasksStore.detail.task.last_error ?? "No reason recorded."}</p>
        </article>
      {/if}

      {#if tasksStore.detail.task.result_metadata}
        <article class="feature-card soft-card">
          <div class="card-head">
            <div>
              <p class="eyebrow">Result Metadata</p>
              <h3>Structured output</h3>
            </div>
          </div>
          <pre>{JSON.stringify(tasksStore.detail.task.result_metadata, null, 2)}</pre>
        </article>
      {/if}

      <div class="stack compact">
        {#each tasksStore.detail.timeline as item}
          <article class="mini-card timeline-item">
            <div class="mini-card-head">
              <strong>{timelineTitle(item)}</strong>
              <span>{item.mode}</span>
            </div>
            <span>{item.event} · {item.status}</span>
            <span>{formatDateTime(item.created_at)}</span>
            {#if item.last_error}
              <p>{item.last_error}</p>
            {/if}
          </article>
        {/each}
      </div>
    {:else}
      <div class="empty-state">
        <h3>Select an execution snapshot</h3>
        <p>Inspect thread approvals, timeline, and result metadata from here.</p>
      </div>
    {/if}
  </section>
</section>

{#if yoloCandidate}
  <div
    class="tasks-mode-modal-backdrop"
    role="presentation"
    tabindex="-1"
    transition:fade={{ duration: 140 }}
    onclick={closeYoloModal}
    onkeydown={(event) => {
      if (event.key === "Escape" || event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        closeYoloModal();
      }
    }}
  >
    <div
      class="tasks-mode-modal"
      role="dialog"
      aria-modal="true"
      aria-label="切换到全自动模式"
      tabindex="-1"
      transition:fly={{ y: 18, duration: 180 }}
      onclick={(event) => event.stopPropagation()}
      onkeydown={(event) => {
        event.stopPropagation();
        if (event.key === "Escape") {
          event.preventDefault();
          closeYoloModal();
        }
      }}
    >
      <div class="tasks-mode-modal-head">
        <div>
          <p class="eyebrow">Yolo Mode</p>
          <h3>切换到全自动模式</h3>
        </div>
      </div>
      <div class="tasks-mode-modal-body">
        <p>
          这会自动批准该任务后续所有原本需要 Ask 的动作；如果它当前正停在审批点，会立即继续执行。
        </p>
      </div>
      <div class="tasks-mode-modal-actions">
        <button class="button button-ghost tasks-mode-action-btn" type="button" onclick={closeYoloModal}>
          继续 Ask
        </button>
        <button class="button button-primary tasks-mode-action-btn" type="button" onclick={confirmYoloMode}>
          继续开启
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .tasks-mode-modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 95;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    background: rgba(12, 17, 26, 0.22);
    backdrop-filter: blur(10px);
  }

  .tasks-mode-modal {
    width: min(100%, 480px);
    display: flex;
    flex-direction: column;
    border-radius: 18px;
    overflow: hidden;
    background: var(--bg-surface);
    border: 1px solid var(--border-default);
    box-shadow: var(--shadow-dropdown);
  }

  .tasks-mode-modal-head {
    padding: 16px 18px 14px;
    border-bottom: 1px solid var(--border-default);
  }

  .tasks-mode-modal h3 {
    margin: 6px 0 0;
    font-size: 18px;
  }

  .tasks-mode-modal-body {
    padding: 16px 18px;
  }

  .tasks-mode-modal-body p {
    margin: 0;
    color: var(--text-secondary);
    line-height: 1.6;
  }

  .tasks-mode-modal-actions {
    padding: 14px 18px 16px;
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    flex-wrap: wrap;
    border-top: 1px solid var(--border-default);
    background: color-mix(in srgb, var(--bg-primary) 44%, transparent);
  }

  .tasks-mode-action-btn {
    min-width: 92px;
    border-radius: 10px;
    padding: 10px 16px;
    font-size: 13px;
    font-weight: 600;
  }
</style>
