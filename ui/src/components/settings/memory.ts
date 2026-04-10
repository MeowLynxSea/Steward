export type MemoryPanelMode = "document" | "daily" | "regression";

export interface MemoryNavItem {
  key: string;
  title: string;
  description: string;
  path?: string;
  kind: MemoryPanelMode;
}

export const memoryGroups: Array<{ title: string; items: MemoryNavItem[] }> = [
  {
    title: "核心记忆",
    items: [
      {
        key: "memory",
        title: "长期记忆",
        description: "MEMORY.md。Agent 的核心记忆库，记录重要事件、决策与经验教训。",
        path: "MEMORY.md",
        kind: "document"
      },
      {
        key: "heartbeat",
        title: "心跳清单",
        description: "HEARTBEAT.md。定期回顾、例行提醒与巡检。",
        path: "HEARTBEAT.md",
        kind: "document"
      },
      {
        key: "daily",
        title: "Daily 日志",
        description: "按天记录的日志，可根据日期快速回顾当天的事件与决策。",
        kind: "daily"
      }
    ]
  },
  {
    title: "身份与上下文",
    items: [
      {
        key: "agents",
        title: "Agent 指令",
        description: "AGENTS.md。定义 Agent 的基本指令集，包括能力、权限与限制。",
        path: "AGENTS.md",
        kind: "document"
      },
      {
        key: "identity",
        title: "身份设定",
        description: "IDENTITY.md。定义 Agent 的身份特征，如角色、背景故事与个性设定。",
        path: "IDENTITY.md",
        kind: "document"
      },
      {
        key: "soul",
        title: "核心价值",
        description: "SOUL.md。定义 Agent 的核心价值观与行为准则，指导其决策与行动。",
        path: "SOUL.md",
        kind: "document"
      },
      {
        key: "user",
        title: "用户画像",
        description: "USER.md。关于用户的偏好、习惯与协作方式。",
        path: "USER.md",
        kind: "document"
      },
      {
        key: "profile",
        title: "心理档案",
        description: "context/profile.json。结构化用户画像源文件。",
        path: "context/profile.json",
        kind: "document"
      },
      {
        key: "directives",
        title: "行为导出",
        description: "context/assistant-directives.md。由 profile 派生出的执行指令。",
        path: "context/assistant-directives.md",
        kind: "document"
      }
    ]
  }
];

export function friendlyTitleForPath(path: string) {
  for (const group of memoryGroups) {
    const item = group.items.find((candidate) => candidate.path === path);
    if (item) {
      return item.title;
    }
  }

  return path.split("/").pop() ?? path;
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
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit"
  }).format(date);
}

export function formatDailyLabel(path: string) {
  const raw = path.split("/").pop()?.replace(/\.md$/, "") ?? path;
  const date = new Date(`${raw}T00:00:00`);

  if (Number.isNaN(date.getTime())) {
    return raw;
  }

  return new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "short",
    day: "numeric",
    weekday: "short"
  }).format(date);
}
