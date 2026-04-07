export type ThemeMode = "light" | "dark";

const STORAGE_KEY = "steward-theme";

class ThemeState {
  mode = $state<ThemeMode>("light");
  initialized = false;

  init() {
    if (this.initialized) {
      this.apply();
      return;
    }

    if (typeof window !== "undefined") {
      const storedMode = window.localStorage.getItem(STORAGE_KEY);
      if (storedMode === "light" || storedMode === "dark") {
        this.mode = storedMode;
      }
    }

    this.initialized = true;
    this.apply();
  }

  setMode(mode: ThemeMode) {
    this.mode = mode;
    this.initialized = true;
    this.apply();
  }

  toggle() {
    this.setMode(this.mode === "dark" ? "light" : "dark");
  }

  private apply() {
    if (typeof document !== "undefined") {
      document.documentElement.setAttribute("data-theme", this.mode);
    }

    if (typeof window !== "undefined") {
      window.localStorage.setItem(STORAGE_KEY, this.mode);
    }
  }
}

export const themeStore = new ThemeState();
