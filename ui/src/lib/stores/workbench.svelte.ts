import { apiClient } from "../api";
import type { WorkbenchCapabilities } from "../types";

class WorkbenchState {
  capabilities = $state<WorkbenchCapabilities | null>(null);
  loading = $state(false);
  error = $state<string | null>(null);
  status = $state("");

  async fetch() {
    this.loading = true;
    this.error = null;
    this.status = "Loading workbench";
    try {
      this.capabilities = await apiClient.getWorkbenchCapabilities();
      this.status = "Workbench ready";
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to load workbench";
      this.status = "Workbench unavailable";
    } finally {
      this.loading = false;
    }
  }
}

export const workbenchStore = new WorkbenchState();
