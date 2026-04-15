type ToastType = "info" | "success" | "error";

interface Toast {
  id: number;
  message: string;
  type: ToastType;
}

let nextId = 0;
let toasts = $state<Toast[]>([]);

export function showToast(message: string, type: ToastType = "info", durationMs = 3000) {
  const id = nextId++;
  toasts.push({ id, message, type });
  setTimeout(() => {
    toasts = toasts.filter((t) => t.id !== id);
  }, durationMs);
}

export function getToasts(): Toast[] {
  return toasts;
}
