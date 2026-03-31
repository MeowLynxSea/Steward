import { apiClient } from "../api";
import type { WorkspaceEntry, WorkspaceSearchResult } from "../types";

class WorkspaceState {
  path = $state("");
  entries = $state<WorkspaceEntry[]>([]);
  searchResults = $state<WorkspaceSearchResult[]>([]);
  searchQuery = $state("");
  loading = $state(false);
  searchLoading = $state(false);
  error = $state<string | null>(null);
  status = $state<string>("");

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
      this.status = `Indexed ${result.path}`;
      await this.refresh();
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to index workspace";
    } finally {
      this.loading = false;
    }
  }
}

export const workspaceStore = new WorkspaceState();
