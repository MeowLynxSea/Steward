import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { StreamEnvelope } from "./types";

const STREAM_EVENT_TYPES = [
  "session:response",
  "session:approval_needed",
  "session:status",
  "session:error",
  "session:stream_chunk",
  "session:thinking",
  "session:tool_started",
  "session:tool_completed",
  "session:tool_result",
  "session:reasoning_update",
  "session:suggestions",
  "session:turn_cost",
  "session:image_generated",
  "session:title_updated",
  "task:created",
  "task:updated",
  "task:waiting_approval",
  "task:mode_changed",
  "task:completed",
  "task:failed",
  "task:rejected"
];

export interface StreamHandle {
  readonly closed: boolean;
  close(): void;
}

/**
 * Creates a typed event stream connection to the backend via Tauri IPC.
 *
 * Lifecycle:
 * - Immediately starts listening to all session events via Tauri `listen()`.
 * - Parses each event as a typed RuntimeEvent and forwards to `onEvent`.
 * - Returns a handle with a `close()` method that shuts down the listener.
 * - Calling `close()` is idempotent.
 *
 * The caller is responsible for calling `close()` when the subscription
 * is no longer needed (e.g. on component destroy or session switch).
 */
export function createEventStream(
  _path: string,
  onEvent: (event: StreamEnvelope) => void
): StreamHandle {
  const unlisteners: UnlistenFn[] = [];
  let closed = false;

  function handlePayload(payload: unknown) {
    try {
      const event = payload as StreamEnvelope;
      onEvent(event);
    } catch {
      // Silently ignore malformed payloads so the stream stays alive.
    }
  }

  // Subscribe to all known event types.
  for (const type of STREAM_EVENT_TYPES) {
    listen(type, (event) => handlePayload(event.payload)).then((unlisten) => {
      if (closed) {
        unlisten();
        return;
      }
      unlisteners.push(unlisten);
    });
  }

  function cleanup() {
    for (const unlisten of unlisteners) {
      unlisten();
    }
    unlisteners.length = 0;
  }

  return {
    get closed() {
      return closed;
    },
    close() {
      if (closed) return;
      closed = true;
      cleanup();
    }
  };
}
