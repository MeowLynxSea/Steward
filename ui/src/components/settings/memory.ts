import type {
  MemoryNodeKind,
  MemorySidebarItem
} from "../../lib/types";

export type MemoryPanelMode = "node" | "search" | "reviews";

export interface MemoryNavItem {
  key: string;
  title: string;
  description: string;
  kind: MemoryPanelMode;
}

export function memoryRouteKey(item: MemorySidebarItem) {
  return item.uri ?? item.node_id;
}

export function memoryKindLabel(kind: MemoryNodeKind) {
  switch (kind) {
    case "boot":
      return "Boot";
    case "identity":
      return "Identity";
    case "value":
      return "Values";
    case "user_profile":
      return "User";
    case "directive":
      return "Directive";
    case "curated":
      return "Curated";
    case "episode":
      return "Episode";
    case "procedure":
      return "Procedure";
    case "reference":
      return "Reference";
    default:
      return kind;
  }
}

export function formatMemoryTimestamp(value: string | null | undefined) {
  if (!value) {
    return "未知时间";
  }

  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }

  return new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit"
  }).format(date);
}

export function routeLabel(route: { domain: string; path: string }) {
  return `${route.domain}://${route.path}`;
}
