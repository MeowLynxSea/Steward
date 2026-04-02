export async function notify(title: string, body: string): Promise<void> {
  try {
    const core = await import("@tauri-apps/api/core");
    await core.invoke("notify", { title, body });
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
  const core = await import("@tauri-apps/api/core");
  return core.invoke<CodexLoginStartResponse>("start_openai_codex_login");
}

export async function getOpenAiCodexLoginStatus(loginId: string): Promise<CodexLoginStatus> {
  const core = await import("@tauri-apps/api/core");
  return core.invoke<CodexLoginStatus>("get_openai_codex_login_status", {
    loginId
  });
}

export async function listenForFolderDrops(
  onDrop: (path: string) => Promise<void> | void
): Promise<() => Promise<void>> {
  try {
    const webviewWindow = await import("@tauri-apps/api/webviewWindow");
    const current = webviewWindow.getCurrentWebviewWindow?.();
    if (!current?.onDragDropEvent) {
      return async () => {};
    }

    const unlisten = await current.onDragDropEvent((event: unknown) => {
      const payload = event as {
        payload?: {
          type?: string;
          paths?: string[];
        };
      };
      if (payload.payload?.type === "drop" && payload.payload.paths?.[0]) {
        void onDrop(payload.payload.paths[0]);
      }
    });

    return async () => {
      unlisten();
    };
  } catch {
    return async () => {};
  }
}
