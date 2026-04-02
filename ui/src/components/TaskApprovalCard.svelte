<script lang="ts">
  import type { TaskRecord } from "../lib/types";

  let {
    task,
    rejectReason = $bindable(),
    showForm = true,
    onApprove,
    onReject
  }: {
    task: TaskRecord;
    rejectReason: string;
    showForm?: boolean;
    onApprove: () => void;
    onReject: () => void;
  } = $props();
</script>

<article class="feature-card soft-card">
  <div class="card-head">
    <div>
      <p class="eyebrow">Pending Approval</p>
      <h3>{task.pending_approval?.risk ?? "Approval"}</h3>
    </div>
  </div>

  <p class="muted">{task.pending_approval?.summary}</p>

  <div class="stack compact">
    {#each task.pending_approval?.operations ?? [] as operation, index}
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
    <button class="button button-primary" onclick={onApprove}>Approve</button>
    <button class="button button-ghost" onclick={onReject}>Reject</button>
  </div>
</article>
