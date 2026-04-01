import { apiClient } from "../api";
import type { WorkspaceEntry, WorkspaceIndexJob, WorkspaceSearchResult } from "../types";

class WorkspaceState {
  path = $state("");
  entries = $state<WorkspaceEntry[]>([]);
  searchResults = $state<WorkspaceSearchResult[]>([]);
  searchQuery = $state("");
  indexJob = $state<WorkspaceIndexJob | null>(null);
  loading = $state(false);
  searchLoading = $state(false);
  error = $state<string | null>(null);
  status = $state<string>("");
  #indexPollTimer: number | null = null;

  async fetch(path = "") {
    this.loading = true;
    this.error = null;
    try {
      const response = await apiClient.getWorkspaceTree(path);
      this.path = response.path;
      this.entries = response.entries;
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to load workspace";
    } finally {
      this.loading = false;
    }
  }

  async refresh() {
    this.error = null;
    try {
      const response = await apiClient.getWorkspaceTree(this.path);
      this.path = response.path;
      this.entries = response.entries;
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to refresh workspace";
    }
  }

  async search(query: string) {
    if (!query.trim()) {
      this.searchResults = [];
      return;
    }
    this.searchLoading = true;
    this.error = null;
    try {
      const response = await apiClient.searchWorkspace(query.trim());
      this.searchResults = response.results;
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Workspace search failed";
    } finally {
      this.searchLoading = false;
    }
  }

  async index(path: string) {
    const trimmed = path.trim();
    if (!trimmed) {
      this.error = "Please enter a folder path to index";
      return;
    }
    this.error = null;
    this.loading = true;
    try {
      const result = await apiClient.indexWorkspace(trimmed);
      this.indexJob = result.job;
      this.status = `Indexing ${result.job.path}`;
      this.#startIndexPolling(result.job.id);
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to index workspace";
    } finally {
      this.loading = false;
    }
  }

  dispose() {
    if (this.#indexPollTimer) {
      window.clearInterval(this.#indexPollTimer);
      this.#indexPollTimer = null;
    }
  }

  #startIndexPolling(jobId: string) {
    if (this.#indexPollTimer) {
      window.clearInterval(this.#indexPollTimer);
    }

    const poll = async () => {
      try {
        const job = await apiClient.getWorkspaceIndexJob(jobId);
        this.indexJob = job;
        this.status = `${job.phase}: ${job.processed_files}/${job.total_files || "?"} files`;

        if (job.status === "completed") {
          if (this.#indexPollTimer) {
            window.clearInterval(this.#indexPollTimer);
            this.#indexPollTimer = null;
          }
          this.status = `Indexed ${job.indexed_files} files from ${job.path}`;
          await this.refresh();
        } else if (job.status === "failed") {
          if (this.#indexPollTimer) {
            window.clearInterval(this.#indexPollTimer);
            this.#indexPollTimer = null;
          }
          this.error = job.error ?? "Workspace indexing failed";
        }
      } catch (e) {
        this.error = e instanceof Error ? e.message : "Failed to read workspace index progress";
      }
    };

    void poll();
    this.#indexPollTimer = window.setInterval(() => {
      void poll();
    }, 1000);
  }
}

export const workspaceStore = new WorkspaceState();
