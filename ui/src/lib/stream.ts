import type { StreamEnvelope } from "./types";

function getApiBase(): string {
  return (globalThis as { __IRONCOWORK_API_BASE__?: string }).__IRONCOWORK_API_BASE__ ?? "/api/v0";
}

const STREAM_EVENT_TYPES = new Set([
  "session.response",
  "session.approval_needed",
  "session.status",
  "session.error",
  "session.stream_chunk",
  "session.thinking",
  "session.tool_started",
  "session.tool_completed",
  "session.tool_result",
  "session.reasoning_update",
  "session.suggestions",
  "session.turn_cost",
  "session.image_generated",
  "task.created",
  "task.updated",
  "task.waiting_approval",
  "task.mode_changed",
  "task.completed",
  "task.failed",
  "task.rejected"
]);

export interface StreamHandle {
  readonly closed: boolean;
  close(): void;
}

/**
 * Creates a typed SSE stream connection to the backend.
 *
 * Lifecycle:
 * - Immediately opens an EventSource to `API_BASE + path`.
 * - Parses each event as a typed RuntimeEvent and forwards to `onEvent`.
 * - Returns a handle with a `close()` method that shuts down the connection.
 * - Calling `close()` is idempotent.
 *
 * The caller is responsible for calling `close()` when the subscription
 * is no longer needed (e.g. on component destroy or session switch).
 */
export function createEventStream(
  path: string,
  onEvent: (event: StreamEnvelope) => void
): StreamHandle {
  const url = `${getApiBase()}${path}`;
  const source = new EventSource(url);
  let closed = false;

  function handleMessage(event: MessageEvent<string>) {
    try {
      const payload = JSON.parse(event.data) as StreamEnvelope;
      onEvent(payload);
    } catch {
      // Silently ignore malformed payloads so the stream stays alive.
    }
  }

  function handleError() {
    // EventSource auto-reconnects by spec.
    // If the stream was explicitly closed, clean up listeners.
    if (closed) {
      cleanup();
    }
  }

  function cleanup() {
    source.removeEventListener("error", handleError as EventListener);
    for (const type of STREAM_EVENT_TYPES) {
      source.removeEventListener(type, handleMessage as EventListener);
    }
  }

  // Subscribe to all known event types.
  for (const type of STREAM_EVENT_TYPES) {
    source.addEventListener(type, handleMessage as EventListener);
  }
  source.addEventListener("error", handleError as EventListener);

  return {
    get closed() {
      return closed;
    },
    close() {
      if (closed) return;
      closed = true;
      cleanup();
      source.close();
    }
  };
}
