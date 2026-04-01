export type View = "sessions" | "tasks" | "workspace" | "settings";

const VALID_VIEWS = new Set<View>(["sessions", "tasks", "workspace", "settings"]);
const DEFAULT_VIEW: View = "sessions";

function parseHash(): View {
  const raw = window.location.hash.replace(/^#\/?/, "");
  if (raw && VALID_VIEWS.has(raw as View)) {
    return raw as View;
  }
  return DEFAULT_VIEW;
}

class RouterState {
  current = $state<View>(parseHash());

  constructor() {
    window.addEventListener("hashchange", () => {
      this.current = parseHash();
    });
  }

  navigate(view: View) {
    window.location.hash = `#/${view}`;
    // Hashchange listener will update `current`.
  }
}

export const router = new RouterState();
