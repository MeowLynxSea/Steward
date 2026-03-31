import { apiClient } from "../api";
import { notify } from "../tauri";
import type { TaskRecord } from "../types";

class TasksState {
  list = $state<TaskRecord[]>([]);
  loading = $state(false);
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
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to refresh tasks";
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
      await apiClient.approveTask(task.id, task.pending_operation?.request_id);
      await this.refresh();
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to approve task";
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
