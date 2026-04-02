import type {
  TaskDetail,
  TaskRecord,
  TaskStatus,
  TaskTimelineEntry
} from "./types";

export function taskStatusTone(status: TaskStatus | string): string {
  switch (status) {
    case "waiting_approval":
      return "warning";
    case "completed":
      return "success";
    case "failed":
    case "rejected":
    case "cancelled":
      return "danger";
    default:
      return "neutral";
  }
}

export function taskStatusCopy(status: TaskStatus | string): string {
  switch (status) {
    case "waiting_approval":
      return "Waiting for approval";
    case "completed":
      return "Completed";
    case "failed":
      return "Failed";
    case "rejected":
      return "Rejected";
    case "cancelled":
      return "Cancelled";
    case "running":
      return "Running";
    default:
      return "Queued";
  }
}

export function timelineTitle(item: { current_step: { title: string } | null; event: string }): string {
  return item.current_step?.title || item.event;
}

export function nextRunFocus(task: TaskRecord | null): string {
  if (!task) return "Start with a goal in the composer.";
  if (task.pending_approval) return task.pending_approval.summary;
  if (task.current_step?.title) return task.current_step.title;
  if (task.last_error) return task.last_error;
  return "Run state is synced from the backend.";
}

export function outlineSteps(detail: TaskDetail | null): string[] {
  if (!detail) return [];

  const titles = detail.timeline
    .map((item) => item.current_step?.title?.trim() ?? "")
    .filter((title) => title.length > 0 && title !== "Queued");

  const deduped: string[] = [];
  for (const title of titles) {
    if (deduped[deduped.length - 1] !== title) {
      deduped.push(title);
    }
  }

  return deduped.slice(-4);
}

export function recentTimeline(detail: TaskDetail | null): TaskTimelineEntry[] {
  if (!detail) return [];
  return [...detail.timeline].slice(-4).reverse();
}

export function resultNotes(detail: TaskDetail | null): string[] {
  const metadata = detail?.task.result_metadata;
  if (!metadata || typeof metadata !== "object") return [];

  const notes: string[] = [];
  const noteValue = metadata.notes;
  if (typeof noteValue === "string" && noteValue.trim()) {
    notes.push(noteValue.trim());
  }

  const artifacts = metadata.artifacts;
  if (Array.isArray(artifacts)) {
    for (const artifact of artifacts) {
      if (
        artifact &&
        typeof artifact === "object" &&
        "path" in artifact &&
        typeof artifact.path === "string" &&
        artifact.path.trim()
      ) {
        notes.push(artifact.path.trim());
      }
    }
  }

  if (notes.length === 0) {
    notes.push(JSON.stringify(metadata, null, 2));
  }

  return notes.slice(0, 4);
}

export function formatDateTime(value: string | null | undefined): string {
  if (!value) return "Unknown";
  return new Date(value).toLocaleString();
}

export function recentRuns(list: TaskRecord[]): TaskRecord[] {
  return list.slice(0, 6);
}
