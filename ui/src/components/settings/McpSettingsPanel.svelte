<script lang="ts">
  import { onMount } from "svelte";
  import { fade, fly } from "svelte/transition";
  import {
    Bot,
    ChevronLeft,
    ChevronRight,
    FolderTree,
    HardDriveUpload,
    KeyRound,
    Network,
    Plus,
    RefreshCcw,
    ScrollText,
    Server,
    ShieldCheck,
    Trash2,
    Wrench
  } from "lucide-svelte";
  import { mcpStore } from "../../lib/stores/mcp.svelte";
  import { sessionsStore } from "../../lib/stores/sessions.svelte";
  import type {
    McpContentBlock,
    McpElicitationRequest,
    McpPrimitiveSchemaDefinition,
    McpPrompt,
    McpSamplingRequest,
    McpServerUpsertRequest,
    TaskRecord
  } from "../../lib/types";

  let { onSeedComposer }: { onSeedComposer: (content: string) => void } = $props();

  type ServerForm = {
    name: string;
    transport: "http" | "stdio" | "unix";
    url: string;
    command: string;
    args: string;
    env: string;
    socket_path: string;
    headers: string;
    enabled: boolean;
    description: string;
    client_id: string;
    authorization_url: string;
    token_url: string;
    scopes: string;
  };

  const EMPTY_FORM: ServerForm = {
    name: "",
    transport: "http",
    url: "",
    command: "",
    args: "",
    env: "",
    socket_path: "",
    headers: "",
    enabled: true,
    description: "",
    client_id: "",
    authorization_url: "",
    token_url: "",
    scopes: ""
  };

  let form = $state<ServerForm>(structuredClone(EMPTY_FORM));
  let rootsText = $state("");
  let promptArgumentValues = $state<Record<string, string>>({});
  let promptSuggestionOpen = $state<Record<string, boolean>>({});
  let resourceTemplateValues = $state<Record<string, Record<string, string>>>({});
  let resourceArgumentSuggestions = $state<Record<string, string[]>>({});
  let resourceSuggestionOpen = $state<Record<string, boolean>>({});
  let samplingOverrides = $state<Record<string, McpSamplingRequest>>({});
  let samplingPreviewDrafts = $state<Record<string, string>>({});
  let elicitationDrafts = $state<Record<string, Record<string, string | number | boolean>>>({});

  type McpSubSection = "servers" | "tools" | "resources" | "prompts" | "approvals" | "roots" | "activity";
  let activeSubSection = $state<McpSubSection | null>(null);

  const subSectionTitle: Record<McpSubSection, string> = {
    servers: "服务配置",
    tools: "工具",
    resources: "资源",
    prompts: "提示词",
    approvals: "审批",
    roots: "路径授权",
    activity: "操作日志"
  };

  let drawerTitle = $derived(
    activeSubSection === "servers"
      ? (mcpStore.selectedServerName ? `编辑：${mcpStore.selectedServerName}` : "添加服务")
      : (activeSubSection ? subSectionTitle[activeSubSection] : "")
  );

  function closeDrawer() {
    activeSubSection = null;
  }

  function formatKeyValueLines(record: Record<string, string>) {
    return Object.entries(record)
      .map(([key, value]) => `${key}=${value}`)
      .join("\n");
  }

  function formatArgumentLines(values: string[]) {
    return values.join("\n");
  }

  function parseKeyValueLines(value: string) {
    return Object.fromEntries(
      value
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean)
        .flatMap((line) => {
          const separator = line.indexOf("=");
          if (separator <= 0) return [];
          const key = line.slice(0, separator).trim();
          const parsedValue = line.slice(separator + 1).trim();
          return key ? [[key, parsedValue] as const] : [];
        })
    );
  }

  function syncFormFromSelection() {
    const selected = mcpStore.selectedServer;
    if (!selected) {
      form = structuredClone(EMPTY_FORM);
      rootsText = "";
      promptArgumentValues = {};
      promptSuggestionOpen = {};
      resourceTemplateValues = {};
      samplingOverrides = {};
      samplingPreviewDrafts = {};
      elicitationDrafts = {};
      return;
    }

    form = {
      ...structuredClone(EMPTY_FORM),
      name: selected.name,
      transport: selected.transport,
      url: selected.url ?? "",
      command: selected.command ?? "",
      args: formatArgumentLines(selected.args ?? []),
      env: formatKeyValueLines(selected.env ?? {}),
      socket_path: selected.socket_path ?? "",
      headers: formatKeyValueLines(selected.headers ?? {}),
      enabled: selected.enabled,
      description: selected.description ?? "",
      client_id: selected.client_id ?? "",
      authorization_url: selected.authorization_url ?? "",
      token_url: selected.token_url ?? "",
      scopes: (selected.scopes ?? []).join(" ")
    };
    rootsText = mcpStore.roots.map((root) => root.uri).join("\n");
    promptArgumentValues = {};
    promptSuggestionOpen = {};
    resourceTemplateValues = {};
    resourceArgumentSuggestions = {};
    resourceSuggestionOpen = {};
    samplingOverrides = {};
    samplingPreviewDrafts = {};
    elicitationDrafts = {};
  }

  function startNewServer() {
    mcpStore.selectedServerName = null;
    form = structuredClone(EMPTY_FORM);
    rootsText = "";
    promptArgumentValues = {};
    promptSuggestionOpen = {};
    resourceTemplateValues = {};
    resourceArgumentSuggestions = {};
    resourceSuggestionOpen = {};
    samplingOverrides = {};
    samplingPreviewDrafts = {};
    elicitationDrafts = {};
    activeSubSection = "servers";
  }

  function editServer(name: string) {
    mcpStore.selectServer(name);
    activeSubSection = "servers";
  }

  function currentPromptArgs(prompt: McpPrompt) {
    return Object.fromEntries(
      prompt.arguments.map((argument) => [argument.name, promptArgumentValues[argument.name] ?? ""])
    );
  }

  function resourceTemplateVariables(uriTemplate: string) {
    return Array.from(uriTemplate.matchAll(/\{([^}]+)\}/g), (match) => match[1]).flatMap((group) =>
      group
        .split(",")
        .map((name) => name.replace(/^[+#./;?&]/, "").replace(/\*$/, "").trim())
        .filter(Boolean)
    );
  }

  function resourceTemplateArgs(uriTemplate: string) {
    return resourceTemplateValues[uriTemplate] ?? {};
  }

  function updateResourceTemplateArg(uriTemplate: string, name: string, value: string) {
    resourceTemplateValues = {
      ...resourceTemplateValues,
      [uriTemplate]: {
        ...(resourceTemplateValues[uriTemplate] ?? {}),
        [name]: value
      }
    };
  }

  function resourceSuggestionKey(uriTemplate: string, argumentName: string) {
    return `${uriTemplate}:${argumentName}`;
  }

  function resourceSuggestions(uriTemplate: string, argumentName: string) {
    return resourceArgumentSuggestions[resourceSuggestionKey(uriTemplate, argumentName)] ?? [];
  }

  async function handleResourceArgumentComplete(uriTemplate: string, argumentName: string) {
    const key = resourceSuggestionKey(uriTemplate, argumentName);
    const suggestions = await mcpStore.completeResourceArgument(
      uriTemplate,
      argumentName,
      resourceTemplateArgs(uriTemplate)[argumentName] ?? "",
      resourceTemplateArgs(uriTemplate)
    );
    resourceArgumentSuggestions = {
      ...resourceArgumentSuggestions,
      [key]: suggestions
    };
    resourceSuggestionOpen = {
      ...resourceSuggestionOpen,
      [key]: suggestions.length > 0
    };
  }

  function applyResourceSuggestion(uriTemplate: string, argumentName: string, value: string) {
    updateResourceTemplateArg(uriTemplate, argumentName, value);
    resourceSuggestionOpen = {
      ...resourceSuggestionOpen,
      [resourceSuggestionKey(uriTemplate, argumentName)]: false
    };
  }

  function instantiateResourceTemplate(uriTemplate: string) {
    const values = resourceTemplateArgs(uriTemplate);
    return uriTemplate.replace(/\{([^}]+)\}/g, (_full, group) => {
      const names = String(group)
        .split(",")
        .map((name) => name.replace(/^[+#./;?&]/, "").replace(/\*$/, "").trim())
        .filter(Boolean);
      if (names.length === 1) {
        return encodeURIComponent(values[names[0]] ?? "");
      }
      return names.map((name) => encodeURIComponent(values[name] ?? "")).join(",");
    });
  }

  function promptSuggestionKey(promptName: string, argumentName: string) {
    return `${promptName}:${argumentName}`;
  }

  function promptSuggestions(promptName: string, argumentName: string) {
    return mcpStore.promptArgumentSuggestions[promptSuggestionKey(promptName, argumentName)] ?? [];
  }

  async function handlePromptArgumentComplete(prompt: McpPrompt, argumentName: string) {
    const key = promptSuggestionKey(prompt.name, argumentName);
    const suggestions = await mcpStore.completePromptArgument(
      prompt.name,
      argumentName,
      promptArgumentValues[argumentName] ?? "",
      currentPromptArgs(prompt)
    );
    promptSuggestionOpen = {
      ...promptSuggestionOpen,
      [key]: suggestions.length > 0
    };
  }

  function applyPromptSuggestion(promptName: string, argumentName: string, value: string) {
    promptArgumentValues = {
      ...promptArgumentValues,
      [argumentName]: value
    };
    promptSuggestionOpen = {
      ...promptSuggestionOpen,
      [promptSuggestionKey(promptName, argumentName)]: false
    };
  }

  function busyFor(prefix: string) {
    return mcpStore.busyAction?.startsWith(prefix) ?? false;
  }

  function samplingRequestFor(taskId: string, request: McpSamplingRequest) {
    return samplingOverrides[taskId] ?? structuredClone(request);
  }

  function samplingPreviewText(task: TaskRecord) {
    const draft = samplingPreviewDrafts[task.id];
    if (draft !== undefined) {
      return draft;
    }
    const preview = task.result_metadata?.preview as
      | { content?: { type?: string; text?: string | null } | null }
      | undefined;
    if (preview?.content?.type === "text" && typeof preview.content.text === "string") {
      return preview.content.text;
    }
    return "";
  }

  function updateSamplingSystemPrompt(taskId: string, request: McpSamplingRequest, value: string) {
    const next = structuredClone(samplingRequestFor(taskId, request));
    next.system_prompt = value;
    samplingOverrides = { ...samplingOverrides, [taskId]: next };
  }

  function updateSamplingMessageText(
    taskId: string,
    request: McpSamplingRequest,
    index: number,
    value: string
  ) {
    const next = structuredClone(samplingRequestFor(taskId, request));
    const block = next.messages[index]?.content;
    if (block?.type !== "text") return;
    next.messages[index] = {
      ...next.messages[index],
      content: {
        ...block,
        text: value
      }
    };
    samplingOverrides = { ...samplingOverrides, [taskId]: next };
  }

  function updateSamplingPreview(taskId: string, value: string) {
    samplingPreviewDrafts = { ...samplingPreviewDrafts, [taskId]: value };
  }

  function schemaEnumValues(schema: McpPrimitiveSchemaDefinition) {
    return schema.type === "string" ? schema.enum ?? [] : [];
  }

  function isSensitiveField(fieldName: string, schema: McpPrimitiveSchemaDefinition) {
    const haystack = `${fieldName} ${schema.title ?? ""} ${schema.description ?? ""}`.toLowerCase();
    return ["password", "secret", "token", "api key", "apikey", "credential", "cookie"].some(
      (needle) => haystack.includes(needle)
    );
  }

  function elicitationDraftFor(taskId: string, request: McpElicitationRequest) {
    if (elicitationDrafts[taskId]) {
      return elicitationDrafts[taskId];
    }
    const draft: Record<string, string | number | boolean> = {};
    for (const [fieldName, schema] of Object.entries(request.requested_schema.properties)) {
      if (schema.type === "boolean") {
        draft[fieldName] = false;
      } else {
        draft[fieldName] = "";
      }
    }
    elicitationDrafts = { ...elicitationDrafts, [taskId]: draft };
    return draft;
  }

  function updateElicitationDraft(
    taskId: string,
    fieldName: string,
    value: string | number | boolean
  ) {
    elicitationDrafts = {
      ...elicitationDrafts,
      [taskId]: {
        ...(elicitationDrafts[taskId] ?? {}),
        [fieldName]: value
      }
    };
  }

  function contentBlockSummary(block: McpContentBlock) {
    switch (block.type) {
      case "text":
        return block.text;
      case "image":
        return `[image ${block.mime_type}]`;
      case "audio":
        return `[audio ${block.mime_type}]`;
      case "resource":
        return JSON.stringify(block.resource, null, 2);
      case "resource_link":
        return `${block.name} · ${block.uri}`;
      default:
        return JSON.stringify(block, null, 2);
    }
  }

  function selectedPromptComposerContent() {
    if (!mcpStore.selectedPrompt) {
      return "";
    }
    return mcpStore.selectedPrompt.messages
      .map((message) => `[${message.role}]\n${contentBlockSummary(message.content)}`)
      .join("\n\n");
  }

  function insertPromptIntoComposer() {
    const content = selectedPromptComposerContent();
    if (!content) return;
    onSeedComposer(content);
  }

  async function runPromptInCurrentThread() {
    const content = selectedPromptComposerContent();
    if (!content || !sessionsStore.activeId || !sessionsStore.active) return;
    await sessionsStore.sendMessage(content);
  }

  async function handleGenerateSampling(task: TaskRecord, request: McpSamplingRequest) {
    const effectiveRequest = samplingRequestFor(task.id, request);
    const nextTask = await mcpStore.respondSampling(task.id, {
      action: "generate",
      request: effectiveRequest
    });
    const preview = nextTask?.result_metadata?.preview as
      | { content?: { type?: string; text?: string | null } | null }
      | undefined;
    if (preview?.content?.type === "text" && typeof preview.content.text === "string") {
      samplingPreviewDrafts = {
        ...samplingPreviewDrafts,
        [task.id]: preview.content.text
      };
    }
  }

  async function handleApproveSampling(task: TaskRecord, request: McpSamplingRequest) {
    await mcpStore.respondSampling(task.id, {
      action: "approve",
      request: samplingRequestFor(task.id, request),
      generated_text: samplingPreviewText(task)
    });
    const { [task.id]: _, ...remainingOverrides } = samplingOverrides;
    samplingOverrides = remainingOverrides;
    const { [task.id]: __, ...remainingDrafts } = samplingPreviewDrafts;
    samplingPreviewDrafts = remainingDrafts;
  }

  function buildElicitationContent(taskId: string, request: McpElicitationRequest) {
    const draft = elicitationDraftFor(taskId, request);
    const content: Record<string, unknown> = {};
    for (const [fieldName, schema] of Object.entries(request.requested_schema.properties)) {
      const value = draft[fieldName];
      if (schema.type === "boolean") {
        content[fieldName] = Boolean(value);
        continue;
      }
      if (value === "" || value === undefined || value === null) {
        continue;
      }
      if (schema.type === "integer") {
        content[fieldName] = Number.parseInt(String(value), 10);
      } else if (schema.type === "number") {
        content[fieldName] = Number.parseFloat(String(value));
      } else {
        content[fieldName] = String(value);
      }
    }
    return content;
  }

  async function handleAcceptElicitation(task: TaskRecord, request: McpElicitationRequest) {
    await mcpStore.respondElicitation(task.id, {
      action: "accept",
      content: buildElicitationContent(task.id, request)
    });
    const { [task.id]: _, ...remainingDrafts } = elicitationDrafts;
    elicitationDrafts = remainingDrafts;
  }

  async function handleSave() {
    const payload: McpServerUpsertRequest = {
      name: form.name.trim(),
      transport: form.transport,
      url: form.transport === "http" ? form.url.trim() : null,
      command: form.transport === "stdio" ? form.command.trim() : null,
      args: form.transport === "stdio" ? form.args.split("\n").map((value) => value.trim()).filter(Boolean) : [],
      env: form.transport === "stdio" ? parseKeyValueLines(form.env) : {},
      socket_path: form.transport === "unix" ? form.socket_path.trim() : null,
      headers: parseKeyValueLines(form.headers),
      enabled: form.enabled,
      description: form.description.trim() || null,
      client_id: form.client_id.trim() || null,
      authorization_url: form.authorization_url.trim() || null,
      token_url: form.token_url.trim() || null,
      scopes: form.scopes
        .split(/[,\s]+/)
        .map((value) => value.trim())
        .filter(Boolean)
    };
    await mcpStore.saveServer(payload);
    syncFormFromSelection();
  }

  async function handleSaveRoots() {
    const roots = rootsText
      .split("\n")
      .map((value) => value.trim())
      .filter(Boolean)
      .map((uri) => ({ uri, name: null }));
    await mcpStore.setRoots(roots);
    rootsText = roots.map((root) => root.uri).join("\n");
  }

  onMount(() => {
    if (mcpStore.servers.length === 0) {
      void mcpStore.fetchOverview();
    } else {
      syncFormFromSelection();
    }
  });

  $effect(() => {
    mcpStore.selectedServerName;
    mcpStore.roots;
    syncFormFromSelection();
  });
</script>

<section class="settings-section">
  <div class="section-header">
    <h4>外部服务</h4>
    <p>连接第三方服务来扩展助手的能力。添加后，助手可使用这些服务提供的工具和数据。</p>
  </div>

  <div class="mcp-toolbar">
    <button class="mini-action primary" type="button" onclick={startNewServer}>
      <Plus size={14} strokeWidth={2} />
      <span>添加服务</span>
    </button>
    <button class="mini-action" type="button" onclick={() => void mcpStore.fetchOverview()}>
      <RefreshCcw size={14} strokeWidth={2} />
      <span>刷新</span>
    </button>
  </div>

  <div class="mcp-server-list">
    {#if mcpStore.servers.length === 0}
      <div class="empty-card">
        <Server size={16} strokeWidth={2} />
        <span>还没有已连接的服务，点击上方「添加服务」开始。</span>
      </div>
    {:else}
      {#each mcpStore.servers as server (server.name)}
        <button
          class:selected={mcpStore.selectedServerName === server.name}
          class="server-pill"
          type="button"
          onclick={() => editServer(server.name)}
        >
          <span class="server-pill-main">
            <strong>{server.name}</strong>
            <small>{server.transport}</small>
          </span>
          <span class="server-pill-meta">
            {#if server.authenticated}
              <ShieldCheck size={13} strokeWidth={2} />
            {/if}
            <span>{server.tool_count} 个工具</span>
          </span>
        </button>
      {/each}
    {/if}
  </div>

  <div class="mcp-nav-list" role="list">
    <button class="mcp-nav-row" type="button" onclick={() => activeSubSection = "tools"}>
      <div class="mcp-nav-row-icon"><Wrench size={15} strokeWidth={2} /></div>
      <div class="mcp-nav-row-main">
        <strong>工具</strong>
        <p>查看服务提供的可用工具</p>
      </div>
      <div class="mcp-nav-row-tail"><ChevronRight size={14} strokeWidth={2} /></div>
    </button>
    <button class="mcp-nav-row" type="button" onclick={() => activeSubSection = "resources"}>
      <div class="mcp-nav-row-icon"><FolderTree size={15} strokeWidth={2} /></div>
      <div class="mcp-nav-row-main">
        <strong>资源</strong>
        <p>浏览和读取服务提供的数据</p>
      </div>
      <div class="mcp-nav-row-tail"><ChevronRight size={14} strokeWidth={2} /></div>
    </button>
    <button class="mcp-nav-row" type="button" onclick={() => activeSubSection = "prompts"}>
      <div class="mcp-nav-row-icon"><Bot size={15} strokeWidth={2} /></div>
      <div class="mcp-nav-row-main">
        <strong>提示词</strong>
        <p>预设的对话模板</p>
      </div>
      <div class="mcp-nav-row-tail"><ChevronRight size={14} strokeWidth={2} /></div>
    </button>
    <button class="mcp-nav-row" type="button" onclick={() => activeSubSection = "approvals"}>
      <div class="mcp-nav-row-icon"><ShieldCheck size={15} strokeWidth={2} /></div>
      <div class="mcp-nav-row-main">
        <strong>审批</strong>
        <p>处理服务发来的确认请求</p>
      </div>
      <div class="mcp-nav-row-tail"><ChevronRight size={14} strokeWidth={2} /></div>
    </button>
    <button class="mcp-nav-row" type="button" onclick={() => activeSubSection = "roots"}>
      <div class="mcp-nav-row-icon"><KeyRound size={15} strokeWidth={2} /></div>
      <div class="mcp-nav-row-main">
        <strong>路径授权</strong>
        <p>管理服务可访问的文件路径</p>
      </div>
      <div class="mcp-nav-row-tail"><ChevronRight size={14} strokeWidth={2} /></div>
    </button>
    <button class="mcp-nav-row" type="button" onclick={() => activeSubSection = "activity"}>
      <div class="mcp-nav-row-icon"><ScrollText size={15} strokeWidth={2} /></div>
      <div class="mcp-nav-row-main">
        <strong>操作日志</strong>
        <p>查看最近的服务操作记录</p>
      </div>
      <div class="mcp-nav-row-tail"><ChevronRight size={14} strokeWidth={2} /></div>
    </button>
  </div>

  {#if mcpStore.status}
    <p class="settings-status">{mcpStore.status}</p>
  {/if}
  {#if mcpStore.error}
    <p class="settings-status error">{mcpStore.error}</p>
  {/if}
</section>

{#if activeSubSection !== null}
  <div
    class="nested-backdrop"
    transition:fade={{ duration: 180 }}
    role="presentation"
    onclick={closeDrawer}
  ></div>

  <div
    class="mcp-drawer"
    in:fly={{ x: -420, duration: 280, easing: (t) => 1 - Math.pow(1 - t, 3) }}
    out:fly={{ x: -420, duration: 220, easing: (t) => t * t }}
  >
    <div class="drawer-header">
      <button class="back-btn" type="button" onclick={closeDrawer} aria-label="返回">
        <ChevronLeft size={18} strokeWidth={2} />
      </button>
      <div class="header-title">
        <p class="header-eyebrow">外部服务</p>
        <h3>{drawerTitle}</h3>
      </div>
    </div>

    <div class="drawer-content mcp-drawer-content">

  {#if activeSubSection === "servers"}
  <div class="settings-card mcp-card">
    <h3>{mcpStore.selectedServer ? `编辑「${mcpStore.selectedServer.name}」` : "添加新服务"}</h3>
    <p class="drawer-intro-text">填写连接信息，测试是否可用，并管理登录认证。</p>

    <div class="mcp-form-grid">
      <label class="model-select">
        <span class="model-select-label">名称</span>
        <input class="model-text-input" bind:value={form.name} placeholder="notion" />
      </label>

      <label class="model-select">
        <span class="model-select-label">连接方式</span>
        <select class="model-select-input" bind:value={form.transport}>
          <option value="http">HTTP（网络请求）</option>
          <option value="stdio">本地进程</option>
          <option value="unix">Unix 套接字</option>
        </select>
      </label>

      {#if form.transport === "http"}
        <label class="model-select">
          <span class="model-select-label">URL</span>
          <input class="model-text-input" bind:value={form.url} placeholder="https://example.com/mcp" />
        </label>
      {:else if form.transport === "stdio"}
        <label class="model-select">
          <span class="model-select-label">启动命令</span>
          <input class="model-text-input" bind:value={form.command} placeholder="npx @modelcontextprotocol/server-filesystem" />
        </label>
        <label class="model-select full">
          <span class="model-select-label">启动参数（每行一个）</span>
          <textarea class="roots-editor compact-editor" bind:value={form.args} placeholder={"dist/index.js\n--stdio"}></textarea>
        </label>
      {:else}
        <label class="model-select">
          <span class="model-select-label">套接字路径</span>
          <input class="model-text-input" bind:value={form.socket_path} placeholder="/tmp/steward.sock" />
        </label>
      {/if}

      <label class="model-select full">
        <span class="model-select-label">描述</span>
        <input class="model-text-input" bind:value={form.description} placeholder="简要说明这个服务的用途" />
      </label>

      {#if form.transport === "stdio"}
        <label class="model-select full">
          <span class="model-select-label">环境变量</span>
          <textarea class="roots-editor compact-editor" bind:value={form.env} placeholder={"NODE_ENV=production\nMCP_LOG=debug"}></textarea>
        </label>
      {/if}

      <label class="model-select full">
        <span class="model-select-label">自定义请求头</span>
        <textarea class="roots-editor compact-editor" bind:value={form.headers} placeholder={"Authorization=Bearer ...\nX-Team=steward"}></textarea>
      </label>

      <label class="model-select">
        <span class="model-select-label">OAuth Client ID</span>
        <input class="model-text-input" bind:value={form.client_id} placeholder="client-id or leave blank for DCR" />
      </label>

      <label class="model-select">
        <span class="model-select-label">授权范围</span>
        <input class="model-text-input" bind:value={form.scopes} placeholder="files.read files.write offline_access" />
      </label>

      <label class="model-select full">
        <span class="model-select-label">授权地址</span>
        <input class="model-text-input" bind:value={form.authorization_url} placeholder="https://provider.example.com/oauth/authorize" />
      </label>

      <label class="model-select full">
        <span class="model-select-label">令牌地址</span>
        <input class="model-text-input" bind:value={form.token_url} placeholder="https://provider.example.com/oauth/token" />
      </label>
    </div>

    <label class="checkbox-row toggle-row">
      <input type="checkbox" bind:checked={form.enabled} />
      <span>启用此服务</span>
    </label>

    <div class="mcp-action-row">
      <button class="mini-action primary" type="button" onclick={() => void handleSave()} disabled={mcpStore.saving}>
        <HardDriveUpload size={14} strokeWidth={2} />
        <span>{mcpStore.saving ? "保存中..." : "保存配置"}</span>
      </button>
      <button
        class="mini-action"
        type="button"
        onclick={() => void mcpStore.testSelectedServer()}
        disabled={!mcpStore.selectedServer || busyFor("test") || busyFor("auth")}
      >
        <Network size={14} strokeWidth={2} />
        <span>测试连接</span>
      </button>
      <button
        class="mini-action"
        type="button"
        onclick={() => void mcpStore.authenticateSelectedServer()}
        disabled={!mcpStore.selectedServer || busyFor("auth")}
      >
        <KeyRound size={14} strokeWidth={2} />
        <span>发起登录</span>
      </button>
      <button
        class="mini-action"
        type="button"
        onclick={() => void mcpStore.finishAuthenticationForSelectedServer()}
        disabled={
          !mcpStore.selectedServer ||
          !mcpStore.selectedServer.requires_auth ||
          mcpStore.selectedServer.authenticated ||
          busyFor("auth")
        }
      >
        <ShieldCheck size={14} strokeWidth={2} />
        <span>完成登录</span>
      </button>
      <button
        class="mini-action danger"
        type="button"
        onclick={() => void mcpStore.deleteSelectedServer()}
        disabled={!mcpStore.selectedServer || busyFor("delete") || busyFor("auth")}
      >
        <Trash2 size={14} strokeWidth={2} />
        <span>删除</span>
      </button>
    </div>

    {#if mcpStore.selectedServer}
      <div class="server-overview-grid">
        <div class="preview-item">
          <strong>协议版本</strong>
          <p>{mcpStore.selectedServer.negotiated_protocol_version ?? "尚未连接"}</p>
        </div>
        <div class="preview-item">
          <strong>最近连通检查</strong>
          <p>
            {mcpStore.selectedServer.last_health_check
              ? new Date(mcpStore.selectedServer.last_health_check).toLocaleString()
              : "尚无记录"}
          </p>
        </div>
        <div class="preview-item">
          <strong>登录状态</strong>
          <p>
            {#if mcpStore.selectedServer.authenticated}
              已登录
            {:else if mcpStore.selectedServer.requires_auth}
              需要登录
            {:else}
              无需登录
            {/if}
          </p>
        </div>
        <div class="preview-item">
          <strong>连接状态</strong>
          <p>{mcpStore.selectedServer.active ? "已连接" : "未连接"}</p>
        </div>
        <div class="preview-item full-span">
          <strong>服务能力</strong>
          <pre>{JSON.stringify(mcpStore.selectedServer.negotiated_capabilities ?? {}, null, 2)}</pre>
        </div>
      </div>
    {/if}
  </div>
  {:else if activeSubSection === "tools"}
  <div class="settings-card mcp-card">
    <p class="drawer-intro-text">当前选中服务提供的工具。助手在对话中会根据需要自动调用。</p>
    <div class="stack-list">
      {#if mcpStore.tools.length === 0}
        <div class="empty-inline">当前没有可用工具。</div>
      {:else}
        {#each mcpStore.tools as tool (tool.name)}
          <div class="stack-row">
            <div>
              <strong>{tool.name}</strong>
              <p>{tool.description || "暂无说明"}</p>
            </div>
            <span class="pill">{tool.annotations?.destructive_hint ? "需审批" : "自动"}</span>
          </div>
        {/each}
      {/if}
    </div>
  </div>
  {:else if activeSubSection === "resources"}
  <div class="settings-card mcp-card">
    <p class="drawer-intro-text">服务提供的数据资源，可以读取内容或订阅变更通知。</p>
    <div class="stack-list">
      {#each mcpStore.resources as resource (resource.uri)}
        {@const subscribed = mcpStore.selectedServer?.subscribed_resource_uris?.includes(resource.uri) ?? false}
        <div class="stack-row">
          <div>
            <strong>{resource.title ?? resource.name}</strong>
            <p>{resource.uri}</p>
          </div>
          <div class="mcp-action-row">
            <button class="mini-action" type="button" onclick={() => void mcpStore.readResource(resource.uri)}>
              <ScrollText size={14} strokeWidth={2} />
              <span>读取</span>
            </button>
            <button
              class="mini-action"
              type="button"
              onclick={() => void mcpStore.addResourceToCurrentThread(resource.uri, sessionsStore.activeId)}
              disabled={!sessionsStore.activeId || !sessionsStore.active || busyFor(`thread-context:${resource.uri}`)}
            >
              <Bot size={14} strokeWidth={2} />
              <span>添加到当前对话</span>
            </button>
            <button
              class:primary={!subscribed}
              class="mini-action"
              type="button"
              onclick={() => void mcpStore.toggleResourceSubscription(resource.uri, subscribed)}
              disabled={busyFor(subscribed ? `unsubscribe:${resource.uri}` : `subscribe:${resource.uri}`)}
            >
              <span>{subscribed ? "取消订阅" : "订阅更新"}</span>
            </button>
            <button
              class="mini-action"
              type="button"
              onclick={() => void mcpStore.saveResourceSnapshot(resource.uri)}
              disabled={busyFor(`snapshot:${resource.uri}`)}
            >
              <HardDriveUpload size={14} strokeWidth={2} />
              <span>保存快照</span>
            </button>
          </div>
        </div>
      {/each}
      {#if mcpStore.resourceTemplates.length > 0}
        <div class="template-divider">模板资源</div>
        {#each mcpStore.resourceTemplates as template (template.uri_template)}
          <div class="stack-row">
            <div>
              <strong>{template.title ?? template.name}</strong>
              <p>{template.uri_template}</p>
              {#if resourceTemplateVariables(template.uri_template).length > 0}
                <div class="template-args">
                  {#each resourceTemplateVariables(template.uri_template) as variableName (`${template.uri_template}:${variableName}`)}
                    {@const suggestionKey = resourceSuggestionKey(template.uri_template, variableName)}
                    <label class="model-select">
                      <span class="model-select-label">{variableName}</span>
                      <div class="prompt-arg-input-row">
                        <input
                          class="model-text-input"
                          value={resourceTemplateArgs(template.uri_template)[variableName] ?? ""}
                          oninput={(event) =>
                            updateResourceTemplateArg(
                              template.uri_template,
                              variableName,
                              (event.currentTarget as HTMLInputElement).value
                            )}
                          placeholder={`Value for ${variableName}`}
                        />
                        <button
                          class="mini-action"
                          type="button"
                          onclick={() => void handleResourceArgumentComplete(template.uri_template, variableName)}
                          disabled={busyFor(`resource-complete:${template.uri_template}:${variableName}`)}
                        >
                          <Wrench size={14} strokeWidth={2} />
                          <span>补全</span>
                        </button>
                      </div>
                      {#if resourceSuggestionOpen[suggestionKey] && resourceSuggestions(template.uri_template, variableName).length > 0}
                        <div class="suggestion-list">
                          {#each resourceSuggestions(template.uri_template, variableName) as suggestion (`${suggestionKey}:${suggestion}`)}
                            <button
                              class="suggestion-pill"
                              type="button"
                              onclick={() => applyResourceSuggestion(template.uri_template, variableName, suggestion)}
                            >
                              {suggestion}
                            </button>
                          {/each}
                        </div>
                      {/if}
                    </label>
                  {/each}
                </div>
              {/if}
            </div>
            <div class="mcp-action-row">
              <button
                class="mini-action"
                type="button"
                onclick={() => void mcpStore.readResource(instantiateResourceTemplate(template.uri_template))}
              >
                <ScrollText size={14} strokeWidth={2} />
                <span>填入参数并读取</span>
              </button>
            </div>
          </div>
        {/each}
      {/if}
      {#if mcpStore.resources.length === 0 && mcpStore.resourceTemplates.length === 0}
        <div class="empty-inline">当前没有可用资源。</div>
      {/if}
    </div>
    {#if mcpStore.selectedResourceContents.length > 0}
      <div class="preview-block">
        {#each mcpStore.selectedResourceContents as content, index (`${content.uri}:${index}`)}
          <div class="preview-item">
            <strong>{content.uri}</strong>
            <pre>{content.text ?? `[binary ${content.mime_type ?? "unknown"}] ${content.blob?.slice(0, 120) ?? ""}`}</pre>
          </div>
        {/each}
      </div>
    {/if}
  </div>
  {:else if activeSubSection === "prompts"}
  <div class="settings-card mcp-card">
    <p class="drawer-intro-text">服务提供的预设提示词。填入参数后可预览内容，或直接在对话中使用。</p>
    <div class="stack-list">
      {#if mcpStore.prompts.length === 0}
        <div class="empty-inline">当前没有可用提示词。</div>
      {:else}
        {#each mcpStore.prompts as prompt (prompt.name)}
          <div class="prompt-card">
            <div class="prompt-header">
              <div>
                <strong>{prompt.title ?? prompt.name}</strong>
                <p>{prompt.description ?? "暂无说明"}</p>
              </div>
              <button class="mini-action" type="button" onclick={() => void mcpStore.resolvePrompt(prompt.name, currentPromptArgs(prompt))}>
                <Bot size={14} strokeWidth={2} />
                <span>预览</span>
              </button>
            </div>
            {#if prompt.arguments.length > 0}
              <div class="prompt-args">
                {#each prompt.arguments as argument (argument.name)}
                  {@const suggestionKey = promptSuggestionKey(prompt.name, argument.name)}
                  <label class="model-select">
                    <span class="model-select-label">{argument.name}</span>
                    <div class="prompt-arg-input-row">
                      <input
                        class="model-text-input"
                        value={promptArgumentValues[argument.name] ?? ""}
                        oninput={(event) => {
                          promptArgumentValues = {
                            ...promptArgumentValues,
                            [argument.name]: (event.currentTarget as HTMLInputElement).value
                          };
                        }}
                        placeholder={argument.description ?? ""}
                      />
                      <button
                        class="mini-action"
                        type="button"
                        onclick={() => void handlePromptArgumentComplete(prompt, argument.name)}
                        disabled={busyFor(`complete:${prompt.name}:${argument.name}`)}
                      >
                        <Wrench size={14} strokeWidth={2} />
                        <span>补全</span>
                      </button>
                    </div>
                    {#if promptSuggestionOpen[suggestionKey] && promptSuggestions(prompt.name, argument.name).length > 0}
                      <div class="suggestion-list">
                        {#each promptSuggestions(prompt.name, argument.name) as suggestion (`${suggestionKey}:${suggestion}`)}
                          <button
                            class="suggestion-pill"
                            type="button"
                            onclick={() => applyPromptSuggestion(prompt.name, argument.name, suggestion)}
                          >
                            {suggestion}
                          </button>
                        {/each}
                      </div>
                    {/if}
                  </label>
                {/each}
              </div>
            {/if}
          </div>
        {/each}
      {/if}
    </div>
    {#if mcpStore.selectedPrompt}
      <div class="preview-block">
        {#if mcpStore.selectedPrompt.description}
          <p class="preview-description">{mcpStore.selectedPrompt.description}</p>
        {/if}
        {#each mcpStore.selectedPrompt.messages as message, index (`${message.role}:${index}`)}
          <div class="preview-item">
            <strong>{message.role}</strong>
            <pre>{message.content.type === "text" ? message.content.text : JSON.stringify(message.content, null, 2)}</pre>
          </div>
        {/each}
        <div class="mcp-action-row">
          <button class="mini-action" type="button" onclick={insertPromptIntoComposer}>
            <Bot size={14} strokeWidth={2} />
            <span>插入到输入框</span>
          </button>
          <button
            class="mini-action primary"
            type="button"
            onclick={() => void runPromptInCurrentThread()}
            disabled={!sessionsStore.activeId || !sessionsStore.active}
          >
            <ShieldCheck size={14} strokeWidth={2} />
            <span>在当前对话中执行</span>
          </button>
        </div>
      </div>
    {/if}
  </div>
  {:else if activeSubSection === "approvals"}
  <div class="settings-card mcp-card">
    <p class="drawer-intro-text">服务发来的请求需要你确认后才会继续执行。</p>

    <div class="approval-grid">
      <div class="approval-column">
        <div class="approval-header">
          <strong>消息生成请求</strong>
          <span class="pill">{mcpStore.samplingApprovals.length}</span>
        </div>
        {#if mcpStore.samplingApprovals.length === 0}
          <div class="empty-inline">当前没有待处理的消息生成请求。</div>
        {:else}
          {#each mcpStore.samplingApprovals as item (item.task.id)}
            {@const request = samplingRequestFor(item.task.id, item.request)}
            <div class="approval-card">
              <div class="approval-card-header">
                <div>
                  <strong>{item.task.title}</strong>
                  <p>{item.task.result_metadata?.server_name ?? mcpStore.selectedServer?.name} · {new Date(item.task.created_at).toLocaleString()}</p>
                </div>
                <span class="pill subtle">{item.task.mode}</span>
              </div>

              <label class="model-select full">
                <span class="model-select-label">系统指令</span>
                <textarea
                  class="roots-editor approval-editor"
                  value={request.system_prompt ?? ""}
                  oninput={(event) =>
                    updateSamplingSystemPrompt(item.task.id, item.request, (event.currentTarget as HTMLTextAreaElement).value)}
                  placeholder="无系统指令"
                ></textarea>
              </label>

              <div class="sampling-message-list">
                {#each request.messages as message, index (`${item.task.id}:${index}`)}
                  <div class="preview-item">
                    <strong>{message.role}</strong>
                    {#if message.content.type === "text"}
                      <textarea
                        class="roots-editor approval-editor"
                        value={message.content.text}
                        oninput={(event) =>
                          updateSamplingMessageText(
                            item.task.id,
                            item.request,
                            index,
                            (event.currentTarget as HTMLTextAreaElement).value
                          )}
                      ></textarea>
                    {:else}
                      <pre>{contentBlockSummary(message.content)}</pre>
                    {/if}
                  </div>
                {/each}
              </div>

              <label class="model-select full">
                <span class="model-select-label">预览 / 回复内容</span>
                <textarea
                  class="roots-editor approval-editor"
                  value={samplingPreviewText(item.task)}
                  oninput={(event) => updateSamplingPreview(item.task.id, (event.currentTarget as HTMLTextAreaElement).value)}
                  placeholder="先点击「生成预览」，再在这里调整最终发送内容"
                ></textarea>
              </label>

              <div class="mcp-action-row">
                <button
                  class="mini-action"
                  type="button"
                  onclick={() => void handleGenerateSampling(item.task, item.request)}
                  disabled={busyFor(`sampling:${item.task.id}:`)}
                >
                  <Bot size={14} strokeWidth={2} />
                  <span>{busyFor(`sampling:${item.task.id}:generate`) ? "生成中..." : "生成预览"}</span>
                </button>
                <button
                  class="mini-action primary"
                  type="button"
                  onclick={() => void handleApproveSampling(item.task, item.request)}
                  disabled={busyFor(`sampling:${item.task.id}:`) || !samplingPreviewText(item.task).trim()}
                >
                  <ShieldCheck size={14} strokeWidth={2} />
                  <span>确认发送</span>
                </button>
                <button
                  class="mini-action"
                  type="button"
                  onclick={() => void mcpStore.respondSampling(item.task.id, { action: "decline" })}
                  disabled={busyFor(`sampling:${item.task.id}:`)}
                >
                  <span>拒绝</span>
                </button>
                <button
                  class="mini-action danger"
                  type="button"
                  onclick={() => void mcpStore.respondSampling(item.task.id, { action: "cancel" })}
                  disabled={busyFor(`sampling:${item.task.id}:`)}
                >
                  <span>取消</span>
                </button>
              </div>
            </div>
          {/each}
        {/if}
      </div>

      <div class="approval-column">
        <div class="approval-header">
          <strong>信息填写请求</strong>
          <span class="pill">{mcpStore.elicitationApprovals.length}</span>
        </div>
        {#if mcpStore.elicitationApprovals.length === 0}
          <div class="empty-inline">当前没有待处理的信息填写请求。</div>
        {:else}
          {#each mcpStore.elicitationApprovals as item (item.task.id)}
            {@const draft = elicitationDraftFor(item.task.id, item.request)}
            <div class="approval-card">
              <div class="approval-card-header">
                <div>
                  <strong>{item.task.title}</strong>
                  <p>{item.task.result_metadata?.server_name ?? mcpStore.selectedServer?.name} · {new Date(item.task.created_at).toLocaleString()}</p>
                </div>
                <span class="pill subtle">{item.task.mode}</span>
              </div>

              <div class="preview-item">
                <strong>服务提示</strong>
                <p>{item.request.message}</p>
              </div>

              <div class="prompt-args">
                {#each Object.entries(item.request.requested_schema.properties) as [fieldName, schema] (`${item.task.id}:${fieldName}`)}
                  <label class="model-select">
                    <span class="model-select-label">
                      {schema.title ?? fieldName}{item.request.requested_schema.required.includes(fieldName) ? " *" : ""}
                    </span>
                    {#if isSensitiveField(fieldName, schema)}
                      <p class="sensitive-note">
                        敏感字段：请确认服务确实需要此信息，再决定是否提交。
                      </p>
                    {/if}
                    {#if schema.type === "boolean"}
                      <label class="checkbox-row toggle-row approval-checkbox">
                        <input
                          type="checkbox"
                          checked={Boolean(draft[fieldName])}
                          onchange={(event) =>
                            updateElicitationDraft(item.task.id, fieldName, (event.currentTarget as HTMLInputElement).checked)}
                        />
                        <span>{schema.description ?? "开关"}</span>
                      </label>
                    {:else if schemaEnumValues(schema).length > 0}
                      <select
                        class="model-select-input"
                        value={String(draft[fieldName] ?? "")}
                        onchange={(event) =>
                          updateElicitationDraft(item.task.id, fieldName, (event.currentTarget as HTMLSelectElement).value)}
                      >
                        <option value="">请选择...</option>
                        {#each schemaEnumValues(schema) as enumValue}
                          <option value={enumValue}>{enumValue}</option>
                        {/each}
                      </select>
                    {:else}
                      <input
                        class="model-text-input"
                        type={schema.type === "number" || schema.type === "integer" ? "number" : "text"}
                        value={String(draft[fieldName] ?? "")}
                        min={schema.type === "number" || schema.type === "integer" ? schema.minimum ?? undefined : undefined}
                        max={schema.type === "number" || schema.type === "integer" ? schema.maximum ?? undefined : undefined}
                        oninput={(event) =>
                          updateElicitationDraft(item.task.id, fieldName, (event.currentTarget as HTMLInputElement).value)}
                        placeholder={schema.description ?? ""}
                      />
                    {/if}
                  </label>
                {/each}
              </div>

              <div class="mcp-action-row">
                <button
                  class="mini-action primary"
                  type="button"
                  onclick={() => void handleAcceptElicitation(item.task, item.request)}
                  disabled={busyFor(`elicitation:${item.task.id}:`)}
                >
                  <ShieldCheck size={14} strokeWidth={2} />
                  <span>提交结果</span>
                </button>
                <button
                  class="mini-action"
                  type="button"
                  onclick={() => void mcpStore.respondElicitation(item.task.id, { action: "decline" })}
                  disabled={busyFor(`elicitation:${item.task.id}:`)}
                >
                  <span>拒绝</span>
                </button>
                <button
                  class="mini-action danger"
                  type="button"
                  onclick={() => void mcpStore.respondElicitation(item.task.id, { action: "cancel" })}
                  disabled={busyFor(`elicitation:${item.task.id}:`)}
                >
                  <span>取消</span>
                </button>
              </div>
            </div>
          {/each}
        {/if}
      </div>
    </div>
  </div>
  {:else if activeSubSection === "roots"}
  <div class="settings-card mcp-card">
    <p class="drawer-intro-text">指定服务可以访问的文件路径，每行一个。</p>
    <textarea class="roots-editor" bind:value={rootsText} placeholder={"file:///Users/me/project\nfile:///Users/me/notes"}></textarea>
    <div class="mcp-action-row">
      <button class="mini-action primary" type="button" onclick={() => void handleSaveRoots()} disabled={!mcpStore.selectedServer}>
        <FolderTree size={14} strokeWidth={2} />
        <span>保存路径</span>
      </button>
    </div>
  </div>
  {:else if activeSubSection === "activity"}
  <div class="settings-card mcp-card">
    <p class="drawer-intro-text">记录最近的服务操作，便于排查问题。</p>
    <div class="stack-list">
      {#if mcpStore.activity.length === 0}
        <div class="empty-inline">当前没有活动记录。</div>
      {:else}
        {#each mcpStore.activity as item (item.id)}
          <div class="stack-row">
            <div>
              <strong>{item.title}</strong>
              <p>{item.server_name} · {item.kind} · {new Date(item.created_at).toLocaleString()}</p>
              {#if item.detail}
                <p>{item.detail}</p>
              {/if}
            </div>
          </div>
        {/each}
      {/if}
    </div>
  </div>
  {/if}

    </div>
  </div>
{/if}

<style>
  /* ─── Base styles (mirrored from SettingsView for scoped CSS) ─── */
  .settings-section {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .section-header h4 {
    margin: 0;
    font-size: 15px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .section-header p {
    margin: 6px 0 0;
    color: var(--text-secondary);
    font-size: 13px;
    line-height: 1.5;
  }

  .settings-status {
    margin: 0;
    font-size: 12px;
    color: var(--text-secondary);
  }

  .settings-status.error {
    color: var(--accent-danger-text);
  }

  .model-select {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .model-select-label {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-secondary);
  }

  .model-select-input {
    width: 100%;
    height: 40px;
    padding: 0 14px;
    padding-right: 38px;
    border-radius: 12px;
    border: 1px solid var(--border-input);
    background: var(--bg-input);
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%23888' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpolyline points='6 9 12 15 18 9'%3E%3C/polyline%3E%3C/svg%3E");
    background-position: right 14px center;
    background-repeat: no-repeat;
    color: var(--text-primary);
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    appearance: none;
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }

  .model-select-input:hover {
    border-color: var(--border-focus, var(--text-tertiary));
  }

  .model-select-input:focus {
    outline: none;
    border-color: var(--accent-primary);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-primary) 20%, transparent);
  }

  .model-text-input {
    width: 100%;
    height: 40px;
    padding: 0 14px;
    border-radius: 12px;
    border: 1px solid var(--border-input);
    background: var(--bg-input);
    color: var(--text-primary);
    font: inherit;
    font-size: 13px;
    font-weight: 500;
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }

  .model-text-input:hover {
    border-color: var(--border-focus, var(--text-tertiary));
  }

  .model-text-input:focus {
    outline: none;
    border-color: var(--accent-primary);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-primary) 20%, transparent);
  }

  /* ─── MCP Panel Styles ─── */
  .mcp-toolbar,
  .mcp-action-row,
  .prompt-header,
  .prompt-arg-input-row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }

  .mcp-server-list,
  .stack-list,
  .prompt-args,
  .preview-block,
  .sampling-message-list,
  .approval-column,
  .suggestion-list,
  .template-args {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .settings-card {
    display: flex;
    flex-direction: column;
    padding: 16px;
    border-radius: 18px;
    border: 1px solid var(--border-default);
    background: var(--bg-surface);
  }

  .server-pill,
  .stack-row,
  .prompt-card,
  .empty-card {
    width: 100%;
    border: 1px solid var(--border-default);
    border-radius: 14px;
    background: var(--bg-surface);
    padding: 12px 14px;
    font-size: 13px;
  }

  .server-pill {
    display: flex;
    align-items: center;
    justify-content: space-between;
    text-align: left;
  }

  .server-pill.selected {
    border-color: color-mix(in srgb, var(--accent-gold) 40%, var(--border-default));
    background: color-mix(in srgb, var(--accent-gold) 8%, var(--bg-surface));
  }

  .server-pill-main,
  .server-pill-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .server-pill-main strong {
    font-size: 13px;
    font-weight: 650;
    color: var(--text-primary);
  }

  .server-pill-main small {
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .server-pill-meta {
    align-items: flex-end;
    color: var(--text-secondary);
    font-size: 11px;
  }

  .mini-action {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 34px;
    border-radius: 12px;
    border: 1px solid var(--border-default);
    background: var(--bg-input);
    color: var(--text-primary);
    padding: 0 12px;
    font: inherit;
    font-size: 12px;
    font-weight: 500;
  }

  .mini-action.primary {
    background: color-mix(in srgb, var(--accent-gold) 14%, var(--bg-input));
  }

  .mini-action.danger {
    color: var(--accent-danger-text);
  }

  .mcp-card {
    gap: 14px;
  }

  .mcp-card h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .mcp-form-grid {
    display: grid;
    gap: 12px;
  }

  .server-overview-grid {
    display: grid;
    gap: 12px;
  }

  .approval-grid {
    display: grid;
    gap: 16px;
  }

  .approval-card {
    display: flex;
    flex-direction: column;
    gap: 12px;
    border: 1px solid var(--border-default);
    border-radius: 14px;
    background: var(--bg-surface);
    padding: 14px;
    font-size: 13px;
  }

  .approval-card-header,
  .approval-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }

  .approval-editor {
    min-height: 92px;
    width: 100%;
    padding: 10px 14px;
    border-radius: 12px;
    border: 1px solid var(--border-input);
    background: var(--bg-input);
    color: var(--text-primary);
    font: inherit;
    font-size: 13px;
    line-height: 1.55;
    resize: vertical;
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }

  .approval-editor:focus {
    outline: none;
    border-color: var(--accent-primary);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-primary) 20%, transparent);
  }

  .compact-editor {
    min-height: 92px;
    width: 100%;
    padding: 10px 14px;
    border-radius: 12px;
    border: 1px solid var(--border-input);
    background: var(--bg-input);
    color: var(--text-primary);
    font: inherit;
    font-size: 13px;
    line-height: 1.55;
    resize: vertical;
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }

  .compact-editor:focus {
    outline: none;
    border-color: var(--accent-primary);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-primary) 20%, transparent);
  }

  .approval-checkbox {
    margin-top: 8px;
  }

  .checkbox-row {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 500;
    color: var(--text-secondary);
  }

  .checkbox-row input[type="checkbox"] {
    width: 16px;
    height: 16px;
    border-radius: 6px;
    border: 1px solid var(--border-input);
    background: var(--bg-input);
    cursor: pointer;
    accent-color: var(--accent-primary);
  }

  .toggle-row {
    align-self: flex-start;
  }

  .mcp-form-grid .full {
    grid-column: 1 / -1;
  }

  .full-span {
    grid-column: 1 / -1;
  }

  .stack-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .stack-row strong,
  .prompt-card strong {
    font-size: 13px;
    font-weight: 650;
    color: var(--text-primary);
  }

  .stack-row p,
  .prompt-card p,
  .preview-description,
  .empty-inline,
  .template-divider {
    margin: 4px 0 0;
    color: var(--text-secondary);
    font-size: 12px;
  }

  .sensitive-note {
    margin: 6px 0 0;
    color: var(--accent-danger-text);
    font-size: 12px;
    line-height: 1.4;
  }

  .pill {
    border-radius: 999px;
    padding: 4px 10px;
    background: color-mix(in srgb, var(--accent-green) 12%, var(--bg-surface) 88%);
    color: var(--text-primary);
    font-size: 12px;
  }

  .pill.subtle {
    background: color-mix(in srgb, var(--accent-gold) 10%, var(--bg-surface) 90%);
  }

  .suggestion-pill {
    display: inline-flex;
    align-items: center;
    justify-content: flex-start;
    width: 100%;
    border-radius: 12px;
    border: 1px solid var(--border-default);
    background: color-mix(in srgb, var(--accent-green) 10%, var(--bg-surface) 90%);
    padding: 8px 10px;
    color: var(--text-primary);
    font: inherit;
    text-align: left;
  }

  .preview-item {
    border-radius: 14px;
    border: 1px solid var(--border-subtle, var(--border-default));
    background: color-mix(in srgb, var(--bg-input) 90%, transparent);
    padding: 12px;
  }

  .preview-item strong {
    font-size: 12px;
    font-weight: 650;
    color: var(--text-secondary);
  }

  .preview-item p {
    margin: 4px 0 0;
    font-size: 13px;
    color: var(--text-primary);
  }

  .preview-item pre {
    margin: 8px 0 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: "SF Mono", "JetBrains Mono", monospace;
    font-size: 12px;
  }

  .roots-editor {
    min-height: 112px;
    width: 100%;
    border-radius: 12px;
    border: 1px solid var(--border-input);
    background: var(--bg-input);
    color: var(--text-primary);
    padding: 10px 14px;
    font: inherit;
    font-size: 13px;
    line-height: 1.55;
    resize: vertical;
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }

  .roots-editor:focus {
    outline: none;
    border-color: var(--accent-primary);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent-primary) 20%, transparent);
  }

  .template-divider {
    margin-top: 4px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  @media (min-width: 960px) {
    .server-overview-grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }

    .approval-grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      align-items: start;
    }
  }

  .mcp-nav-list {
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border-subtle, var(--border-default));
  }

  .mcp-nav-row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px 0;
    border: none;
    border-bottom: 1px solid var(--border-subtle, var(--border-default));
    background: transparent;
    text-align: left;
    color: inherit;
    cursor: pointer;
  }

  .mcp-nav-row-icon {
    width: 30px;
    height: 30px;
    border-radius: 10px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--text-tertiary);
    background: color-mix(in srgb, var(--bg-input) 88%, transparent);
    flex-shrink: 0;
  }

  .mcp-nav-row-main {
    min-width: 0;
    flex: 1;
  }

  .mcp-nav-row-main strong {
    font-size: 14px;
    font-weight: 650;
    color: var(--text-primary);
  }

  .mcp-nav-row-main p {
    margin: 4px 0 0;
    font-size: 12px;
    line-height: 1.6;
    color: var(--text-secondary);
  }

  .mcp-nav-row-tail {
    width: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--text-tertiary);
    flex-shrink: 0;
  }

  .mcp-nav-row:hover .mcp-nav-row-icon {
    color: var(--accent-primary);
    background: color-mix(in srgb, var(--accent-primary) 10%, var(--bg-input));
  }

  .mcp-nav-row:hover .mcp-nav-row-tail {
    color: var(--text-primary);
    transform: translateX(2px);
  }

  .nested-backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--settings-z-nested-backdrop, 92);
    background: rgba(0, 0, 0, 0.15);
    backdrop-filter: blur(8px);
  }

  .mcp-drawer {
    position: fixed;
    top: 0;
    bottom: 0;
    left: 0;
    z-index: var(--settings-z-nested-drawer, 93);
    width: min(420px, 100vw);
    display: flex;
    flex-direction: column;
    background: var(--bg-surface);
    border-right: 1px solid var(--border-default);
    box-shadow: var(--shadow-dropdown);
    overflow: hidden;
  }

  .drawer-header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 18px 20px;
    border-bottom: 1px solid var(--border-subtle, var(--border-default));
  }

  .back-btn {
    width: 34px;
    height: 34px;
    border-radius: 12px;
    border: 1px solid var(--border-subtle, var(--border-default));
    background: var(--bg-input);
    color: var(--text-primary);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }

  .header-title {
    min-width: 0;
  }

  .header-eyebrow {
    margin: 0 0 4px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--text-muted);
  }

  .header-title h3 {
    margin: 0;
    font-size: 17px;
    font-weight: 700;
    color: var(--text-primary);
  }

  .mcp-drawer-content {
    min-height: 0;
    flex: 1;
    display: flex;
    flex-direction: column;
    padding: 18px 20px 22px;
    gap: 14px;
    overflow-y: auto;
  }

  .drawer-intro-text {
    margin: 0;
    font-size: 13px;
    line-height: 1.55;
    color: var(--text-secondary);
  }

  @media (max-width: 640px) {
    .mcp-drawer {
      width: 100vw;
    }
  }
</style>
