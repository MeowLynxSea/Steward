import { apiClient } from "../api";
import { notify } from "../tauri";
import type { TaskDetail, TaskRecord } from "../types";

class TasksState {
  list = $state<TaskRecord[]>([]);
  activeId = $state<string | null>(null);
  detail = $state<TaskDetail | null>(null);
  loading = $state(false);
  detailLoading = $state(false);
  error = $state<string | null>(null);
  status = $state<string>("");
  #previousStates = new Map<string, string>();

  async fetch() {
    this.loading = true;
    this.error = null;
    try {
      const response = await apiClient.listTasks();
      this.#notifyChanges(response.tasks);
      this.list = response.tasks;
      if (!this.activeId || !this.list.some((task) => task.id === this.activeId)) {
        this.activeId = this.list[0]?.id ?? null;
      }
      if (this.activeId) {
        await this.select(this.activeId);
      }
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to load tasks";
    } finally {
      this.loading = false;
    }
  }

  async refresh() {
    this.error = null;
    try {
      const response = await apiClient.listTasks();
      this.#notifyChanges(response.tasks);
      this.list = response.tasks;
      if (!this.activeId || !this.list.some((task) => task.id === this.activeId)) {
        this.activeId = this.list[0]?.id ?? null;
      }
      if (this.activeId) {
        await this.select(this.activeId);
      }
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to refresh tasks";
    }
  }

  async select(id: string) {
    this.activeId = id;
    this.detailLoading = true;
    this.error = null;
    try {
      this.detail = await apiClient.getTask(id);
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to load task detail";
    } finally {
      this.detailLoading = false;
    }
  }

  async createArchiveTask(sourcePath: string, targetRoot: string, mode: "ask" | "yolo") {
    this.error = null;
    try {
      const response = await apiClient.createTask({
        template_id: "builtin:file-archive",
        mode,
        parameters: {
          source_path: sourcePath,
          target_root: targetRoot,
          naming_strategy: "preserve",
          exclude_patterns: []
        }
      });
      await this.refresh();
      await this.select(response.task_id);
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to create archive task";
    }
  }

  async toggleMode(task: TaskRecord) {
    this.error = null;
    try {
      const nextMode = task.mode === "yolo" ? "ask" : "yolo";
      await apiClient.patchTaskMode(task.id, nextMode);
      await this.refresh();
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to toggle task mode";
    }
  }

  async approve(task: TaskRecord) {
    this.error = null;
    try {
      await apiClient.approveTask(task.id, task.pending_approval?.id);
      await this.refresh();
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to approve task";
    }
  }

  async reject(task: TaskRecord) {
    this.error = null;
    try {
      await apiClient.rejectTask(task.id, task.pending_approval?.id, "rejected by user");
      await this.refresh();
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to reject task";
    }
  }

  #notifyChanges(tasks: TaskRecord[]) {
    for (const task of tasks) {
      const previous = this.#previousStates.get(task.id);
      if (previous && previous !== task.status) {
        if (task.status === "waiting_approval") {
          void notify("Task waiting for approval", task.title);
        }
        if (task.status === "completed") {
          void notify("Task completed", task.title);
        }
      }
      this.#previousStates.set(task.id, task.status);
    }
  }
}

export const tasksStore = new TasksState();
