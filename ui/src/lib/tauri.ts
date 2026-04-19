import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

export async function notify(title: string, body: string): Promise<void> {
  try {
    await invoke("notify", { title, body });
  } catch {
    // Browser mode: no native notification bridge.
  }
}

export interface CodexLoginStartResponse {
  login_id: string;
  verification_uri: string;
  user_code: string;
}

export type CodexLoginStatus =
  | {
      status: "pending";
      verification_uri: string;
      user_code: string;
    }
  | {
      status: "success";
    }
  | {
      status: "error";
      message: string;
    };

export async function startOpenAiCodexLogin(): Promise<CodexLoginStartResponse> {
  return invoke<CodexLoginStartResponse>("start_openai_codex_login");
}

export async function getOpenAiCodexLoginStatus(loginId: string): Promise<CodexLoginStatus> {
  return invoke<CodexLoginStatus>("get_openai_codex_login_status", {
    loginId
  });
}

export async function listenForFolderDrops(
  onDrop: (path: string) => Promise<void> | void
): Promise<() => void> {
  try {
    const current = getCurrentWebviewWindow?.();
    if (!current?.onDragDropEvent) {
      return () => {};
    }

    const unlisten = await current.onDragDropEvent((event: unknown) => {
      const payload = event as {
        payload?: {
          type?: string;
          paths?: string[];
        };
      };
      if (payload.payload?.type === "drop" && payload.payload.paths?.[0]) {
        void (async () => {
          const path = payload.payload?.paths?.[0];
          if (!path) {
            return;
          }
          if (!(await isDirectoryPath(path))) {
            return;
          }
          await onDrop(path);
        })();
      }
    });

    return () => {
      unlisten();
    };
  } catch {
    return () => {};
  }
}

export interface WindowFileDropEvent {
  type: "enter" | "over" | "drop" | "leave";
  paths: string[];
  position: {
    x: number;
    y: number;
  } | null;
}

export async function listenForFileDrops(
  onEvent: (event: WindowFileDropEvent) => Promise<void> | void
): Promise<() => void> {
  try {
    const current = getCurrentWebviewWindow?.();
    if (!current?.onDragDropEvent) {
      return () => {};
    }

    const unlisten = await current.onDragDropEvent((event: unknown) => {
      const payload = event as {
        payload?: {
          type?: "enter" | "over" | "drop" | "leave";
          paths?: string[];
          position?: {
            x?: number;
            y?: number;
          };
        };
      };
      const type = payload.payload?.type;
      if (!type) {
        return;
      }

      void onEvent({
        type,
        paths: payload.payload?.paths ?? [],
        position: payload.payload?.position
          ? {
              x: payload.payload.position.x ?? 0,
              y: payload.payload.position.y ?? 0
            }
          : null
      });
    });

    return () => {
      unlisten();
    };
  } catch {
    return () => {};
  }
}

export async function pickDirectory(): Promise<string | null> {
  try {
    const selection = await invoke<string | null>("pick_allowlist_directory");
    return selection ?? null;
  } catch {
    return null;
  }
}

async function isDirectoryPath(path: string): Promise<boolean> {
  try {
    return await invoke<boolean>("path_is_directory", { path });
  } catch {
    return false;
  }
}
