import { apiClient } from "../api";
import type { TaskTemplateRecord } from "../types";

class TemplatesState {
  list = $state<TaskTemplateRecord[]>([]);
  activeId = $state<string | null>(null);
  loading = $state(false);
  error = $state<string | null>(null);
  status = $state("");

  get active(): TaskTemplateRecord | null {
    return this.list.find((template) => template.id === this.activeId) ?? null;
  }

  async fetch() {
    this.loading = true;
    this.error = null;
    this.status = "Loading templates...";
    try {
      const response = await apiClient.listTemplates();
      this.list = response.templates;
      if (!this.activeId || !this.list.some((template) => template.id === this.activeId)) {
        this.activeId = this.list[0]?.id ?? null;
      }
      this.status = this.list.length > 0 ? "Templates loaded" : "No templates yet";
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to load templates";
      this.status = "Template load failed";
    } finally {
      this.loading = false;
    }
  }

  select(id: string) {
    this.activeId = id;
  }
}

export const templatesStore = new TemplatesState();
