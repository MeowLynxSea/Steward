<script lang="ts">
  import StatusBadge from "../components/StatusBadge.svelte";
  import TaskApprovalCard from "../components/TaskApprovalCard.svelte";
  import { formatDateTime, timelineTitle } from "../lib/presentation";
  import { tasksStore } from "../lib/stores/tasks.svelte";

  let rejectReason = $state("Rejected by user");
</script>

<section class="view-grid split-grid">
  <section class="panel column-panel">
    <div class="card-head">
      <div>
        <p class="eyebrow">Runs</p>
        <h2>Execution queue</h2>
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
      <p class="muted">Loading runs...</p>
    {:else if tasksStore.list.length === 0}
      <p class="muted">No runs yet. Start from the session workbench.</p>
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
          <p class="eyebrow">Run Detail</p>
          <h2>{tasksStore.detail.task.title}</h2>
          <p class="muted">{tasksStore.detail.task.id}</p>
        </div>

        <div class="hero-status">
          <StatusBadge status={tasksStore.detail.task.status} />
          <p>{tasksStore.detail.task.current_step?.title ?? "Run state updated"}</p>
        </div>
      </div>

      <div class="toolbar">
        <button class="button button-ghost" onclick={() => void tasksStore.toggleMode(tasksStore.detail!.task)}>
          {tasksStore.detail.task.mode === "yolo" ? "Switch To Ask" : "Switch To Yolo"}
        </button>
        {#if !["completed", "failed", "rejected", "cancelled"].includes(tasksStore.detail.task.status)}
          <button class="button button-ghost" onclick={() => void tasksStore.cancel(tasksStore.detail!.task)}>Cancel</button>
        {/if}
        {#if tasksStore.detail.task.status === "waiting_approval" && tasksStore.detail.task.pending_approval}
          <button class="button button-primary" onclick={() => void tasksStore.approve(tasksStore.detail!.task)}>Approve</button>
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
        <h3>Select a run</h3>
        <p>Inspect its timeline, approvals, and result metadata from here.</p>
      </div>
    {/if}
  </section>
</section>
