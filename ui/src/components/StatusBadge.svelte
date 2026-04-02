<script lang="ts">
  import { taskStatusCopy, taskStatusTone } from "../lib/presentation";

  let { status }: { status: string } = $props();
</script>

<span class={`status-badge ${taskStatusTone(status)}`}>
  {taskStatusCopy(status)}
</span>
