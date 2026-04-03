import { apiClient } from "../api";
import { createEventStream, type StreamHandle } from "../stream";
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
  #detailStream: StreamHandle | null = null;

  get pendingApprovals() {
    return this.list.filter((task) => task.status === "waiting_approval");
  }

  get recentDecisions() {
    return this.list.filter((task) =>
      task.status === "completed" ||
      task.status === "rejected" ||
      task.status === "failed" ||
      task.status === "cancelled"
    );
  }

  async fetch() {
    this.loading = true;
    this.error = null;
    try {
      this.status = "Loading tasks";
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
      this.status = "";
      this.loading = false;
    }
  }

  async refresh() {
    this.error = null;
    try {
      this.status = "Refreshing tasks";
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
    } finally {
      this.status = "";
    }
  }

  async select(id: string) {
    this.activeId = id;
    this.detailLoading = true;
    this.error = null;
    try {
      this.detail = await apiClient.getTask(id);
      this.#connectDetailStream(id);
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to load task detail";
    } finally {
      this.detailLoading = false;
    }
  }

  async toggleMode(task: TaskRecord) {
    this.error = null;
    try {
      this.status = `Switching ${task.title} to ${task.mode === "yolo" ? "ask" : "yolo"}`;
      const nextMode = task.mode === "yolo" ? "ask" : "yolo";
      await apiClient.patchTaskMode(task.id, nextMode);
      await this.#refreshActiveTask();
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to toggle task mode";
    } finally {
      this.status = "";
    }
  }

  async approve(task: TaskRecord, always = false) {
    this.error = null;
    try {
      this.status = `${always ? "Always allowing" : "Approving"} ${task.title}`;
      await apiClient.approveTask(task.id, task.pending_approval?.id, always);
      await this.#refreshActiveTask();
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to approve task";
    } finally {
      this.status = "";
    }
  }

  async reject(task: TaskRecord, reason: string) {
    this.error = null;
    try {
      this.status = `Rejecting ${task.title}`;
      await apiClient.rejectTask(
        task.id,
        task.pending_approval?.id,
        reason.trim() || "rejected by user"
      );
      await this.#refreshActiveTask();
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to reject task";
    } finally {
      this.status = "";
    }
  }

  async cancel(task: TaskRecord) {
    this.error = null;
    try {
      this.status = `Cancelling ${task.title}`;
      await apiClient.cancelTask(task.id);
      await this.#refreshActiveTask();
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to cancel task";
    } finally {
      this.status = "";
    }
  }

  dispose() {
    this.#detailStream?.close();
    this.#detailStream = null;
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

  async #refreshActiveTask() {
    if (this.activeId) {
      await this.select(this.activeId);
      return;
    }
    await this.refresh();
  }

  #connectDetailStream(taskId: string) {
    if (this.#detailStream && this.activeId === taskId && !this.#detailStream.closed) {
      return;
    }
    this.#detailStream?.close();
    this.#detailStream = createEventStream(`/tasks/${taskId}/stream`, () => {
      void this.#syncTaskDetail(taskId);
    });
  }

  async #syncTaskDetail(taskId: string) {
    if (this.activeId !== taskId) {
      return;
    }

    try {
      const detail = await apiClient.getTask(taskId);
      this.detail = detail;
      this.list = this.list.map((task) => (task.id === taskId ? detail.task : task));
      this.#notifyChanges(this.list);
    } catch (e) {
      this.error = e instanceof Error ? e.message : "Failed to sync task updates";
    }
  }
}

export const tasksStore = new TasksState();
