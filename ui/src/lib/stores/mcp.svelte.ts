import { apiClient } from "../api";
import type {
  McpActivityItem,
  McpElicitationRequest,
  McpPrompt,
  McpPromptResponse,
  McpResource,
  McpResourceTemplate,
  McpRootGrant,
  McpSamplingRequest,
  McpServerSummary,
  McpServerUpsertRequest,
  McpTool,
  TaskRecord
} from "../types";

function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message) {
    return error.message;
  }
  if (typeof error === "string" && error.trim()) {
    return error;
  }
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof (error as { message?: unknown }).message === "string"
  ) {
    return (error as { message: string }).message;
  }
  return fallback;
}

class McpState {
  loading = $state(false);
  saving = $state(false);
  busyAction = $state<string | null>(null);
  error = $state<string | null>(null);
  status = $state("");

  servers = $state<McpServerSummary[]>([]);
  selectedServerName = $state<string | null>(null);
  tools = $state<McpTool[]>([]);
  resources = $state<McpResource[]>([]);
  resourceTemplates = $state<McpResourceTemplate[]>([]);
  prompts = $state<McpPrompt[]>([]);
  roots = $state<McpRootGrant[]>([]);
  activity = $state<McpActivityItem[]>([]);
  pendingTasks = $state<TaskRecord[]>([]);
  selectedPrompt = $state<McpPromptResponse["prompt"] | null>(null);
  selectedResourceContents = $state<Array<{ uri: string; mime_type?: string | null; text?: string; blob?: string }>>([]);
  promptArgumentSuggestions = $state<Record<string, string[]>>({});

  get selectedServer() {
    return this.servers.find((server) => server.name === this.selectedServerName) ?? null;
  }

  get samplingApprovals() {
    return this.pendingTasks.reduce<Array<{ task: TaskRecord; request: McpSamplingRequest }>>(
      (items, task) => {
        if (task.template_id !== "mcp:sampling") {
          return items;
        }
        const serverName = task.result_metadata?.server_name;
        if (this.selectedServerName && serverName !== this.selectedServerName) {
          return items;
        }
        const request = (task.result_metadata?.request ?? null) as McpSamplingRequest | null;
        if (request) {
          items.push({ task, request });
        }
        return items;
      },
      []
    );
  }

  get elicitationApprovals() {
    return this.pendingTasks.reduce<Array<{ task: TaskRecord; request: McpElicitationRequest }>>(
      (items, task) => {
        if (task.template_id !== "mcp:elicitation") {
          return items;
        }
        const serverName = task.result_metadata?.server_name;
        if (this.selectedServerName && serverName !== this.selectedServerName) {
          return items;
        }
        const request = (task.result_metadata?.request ?? null) as McpElicitationRequest | null;
        if (request) {
          items.push({ task, request });
        }
        return items;
      },
      []
    );
  }

  async fetchOverview() {
    this.loading = true;
    this.error = null;
    try {
      const [serversResponse, activityResponse, tasksResponse] = await Promise.all([
        apiClient.listMcpServers(),
        apiClient.listMcpActivity(),
        apiClient.listTasks()
      ]);
      this.servers = serversResponse.servers;
      this.activity = activityResponse.items;
      this.pendingTasks = tasksResponse.tasks.filter(
        (task) =>
          task.status === "waiting_approval" &&
          (task.template_id === "mcp:sampling" || task.template_id === "mcp:elicitation")
      );
      if (!this.selectedServerName && this.servers.length > 0) {
        this.selectedServerName = this.servers[0].name;
      }
      if (this.selectedServerName && !this.servers.some((server) => server.name === this.selectedServerName)) {
        this.selectedServerName = this.servers[0]?.name ?? null;
      }
      if (this.selectedServerName) {
        await this.refreshSelectedServer();
      }
      this.status = this.servers.length > 0 ? "MCP servers loaded" : "No MCP servers configured yet";
    } catch (error) {
      this.error = errorMessage(error, "Failed to load MCP servers");
    } finally {
      this.loading = false;
    }
  }

  async refreshSelectedServer() {
    const name = this.selectedServerName;
    if (!name) {
      this.tools = [];
      this.resources = [];
      this.resourceTemplates = [];
      this.prompts = [];
      this.roots = [];
      this.pendingTasks = [];
      this.selectedPrompt = null;
      this.selectedResourceContents = [];
      this.promptArgumentSuggestions = {};
      return;
    }

    this.error = null;
    try {
      const serversResponse = await apiClient.listMcpServers();
      this.servers = serversResponse.servers;
      if (this.selectedServerName && !this.servers.some((server) => server.name === this.selectedServerName)) {
        this.selectedServerName = this.servers[0]?.name ?? null;
      }
      const selectedName = this.selectedServerName;
      if (!selectedName) {
        return;
      }
      const [
        toolsResponse,
        resourcesResponse,
        templatesResponse,
        promptsResponse,
        rootsResponse,
        activityResponse,
        tasksResponse
      ] =
        await Promise.all([
          apiClient.listMcpTools(selectedName),
          apiClient.listMcpResources(selectedName),
          apiClient.listMcpResourceTemplates(selectedName),
          apiClient.listMcpPrompts(selectedName),
          apiClient.getMcpRoots(selectedName),
          apiClient.listMcpActivity(),
          apiClient.listTasks()
        ]);

      this.tools = toolsResponse.tools;
      this.resources = resourcesResponse.resources;
      this.resourceTemplates = templatesResponse.templates;
      this.prompts = promptsResponse.prompts;
      this.roots = rootsResponse.roots;
      this.activity = activityResponse.items;
      this.promptArgumentSuggestions = {};
      this.pendingTasks = tasksResponse.tasks.filter(
        (task) =>
          task.status === "waiting_approval" &&
          (task.template_id === "mcp:sampling" || task.template_id === "mcp:elicitation")
      );
    } catch (error) {
      this.error = errorMessage(error, "Failed to load MCP server details");
    }
  }

  selectServer(name: string) {
    this.selectedServerName = name;
    void this.refreshSelectedServer();
  }

  async saveServer(payload: McpServerUpsertRequest) {
    this.saving = true;
    this.error = null;
    this.status = "";
    try {
      const response = await apiClient.upsertMcpServer(payload);
      this.selectedServerName = response.server.name;
      await this.fetchOverview();
      this.status = `Saved MCP server ${response.server.name}`;
      return true;
    } catch (error) {
      this.error = errorMessage(error, "Failed to save MCP server");
      return false;
    } finally {
      this.saving = false;
    }
  }

  async deleteSelectedServer() {
    const name = this.selectedServerName;
    if (!name) return;
    this.busyAction = "delete";
    this.error = null;
    this.status = "";
    try {
      const message = await apiClient.deleteMcpServer(name);
      this.status = message;
      this.selectedServerName = null;
      await this.fetchOverview();
    } catch (error) {
      this.error = errorMessage(error, "Failed to delete MCP server");
    } finally {
      this.busyAction = null;
    }
  }

  async testSelectedServer() {
    const name = this.selectedServerName;
    if (!name) return;
    this.busyAction = "test";
    this.error = null;
    try {
      const response = await apiClient.testMcpServer(name);
      this.status = response.message;
      await this.fetchOverview();
    } catch (error) {
      this.error = errorMessage(error, "Failed to test MCP server");
    } finally {
      this.busyAction = null;
    }
  }

  async authenticateSelectedServer() {
    const name = this.selectedServerName;
    if (!name) return;
    this.busyAction = "auth";
    this.error = null;
    try {
      const response = await apiClient.beginMcpAuth(name);
      this.status = response.message;
      await this.fetchOverview();
    } catch (error) {
      this.error = errorMessage(error, "Failed to authenticate MCP server");
    } finally {
      this.busyAction = null;
    }
  }

  async finishAuthenticationForSelectedServer() {
    const name = this.selectedServerName;
    if (!name) return;
    this.busyAction = "auth-finish";
    this.error = null;
    try {
      const response = await apiClient.finishMcpAuth(name);
      this.status = response.message;
      await this.fetchOverview();
    } catch (error) {
      this.error = errorMessage(error, "Failed to finish MCP authentication");
    } finally {
      this.busyAction = null;
    }
  }

  async readResource(uri: string) {
    const name = this.selectedServerName;
    if (!name) return;
    this.busyAction = "read-resource";
    this.error = null;
    try {
      const response = await apiClient.readMcpResource(name, uri);
      this.selectedResourceContents = response.resource.contents;
      this.status = `Loaded resource ${uri}`;
      await this.refreshSelectedServer();
    } catch (error) {
      this.error = errorMessage(error, "Failed to read MCP resource");
    } finally {
      this.busyAction = null;
    }
  }

  async saveResourceSnapshot(uri: string) {
    const name = this.selectedServerName;
    if (!name) return null;
    this.busyAction = `snapshot:${uri}`;
    this.error = null;
    try {
      const response = await apiClient.saveMcpResourceSnapshot(name, uri);
      this.status = `Saved MCP snapshot to ${response.snapshot_path}`;
      await this.refreshSelectedServer();
      return response.snapshot_path;
    } catch (error) {
      this.error = errorMessage(error, "Failed to save MCP resource snapshot");
      return null;
    } finally {
      this.busyAction = null;
    }
  }

  async addResourceToCurrentThread(uri: string, sessionId: string) {
    const name = this.selectedServerName;
    if (!name) return null;
    this.busyAction = `thread-context:${uri}`;
    this.error = null;
    try {
      const response = await apiClient.addMcpResourceToThreadContext(sessionId, name, uri);
      this.status = `Added ${response.attachment_count} MCP resource attachment(s) to the current thread`;
      await this.refreshSelectedServer();
      return response;
    } catch (error) {
      this.error = errorMessage(error, "Failed to add MCP resource to thread context");
      return null;
    } finally {
      this.busyAction = null;
    }
  }

  async resolvePrompt(name: string, argumentsMap: Record<string, string>) {
    const serverName = this.selectedServerName;
    if (!serverName) return;
    this.busyAction = "resolve-prompt";
    this.error = null;
    try {
      const response = await apiClient.getMcpPrompt(serverName, name, {
        arguments: argumentsMap
      });
      this.selectedPrompt = response.prompt;
      this.status = `Loaded prompt ${name}`;
      await this.refreshSelectedServer();
    } catch (error) {
      this.error = errorMessage(error, "Failed to resolve MCP prompt");
    } finally {
      this.busyAction = null;
    }
  }

  async completePromptArgument(
    promptName: string,
    argumentName: string,
    value: string,
    contextArguments: Record<string, string>
  ) {
    const serverName = this.selectedServerName;
    if (!serverName) return [];
    this.busyAction = `complete:${promptName}:${argumentName}`;
    this.error = null;
    try {
      const response = await apiClient.completeMcpArgument(serverName, {
        reference_type: "prompt",
        reference_name: promptName,
        argument_name: argumentName,
        value,
        context_arguments: contextArguments
      });
      this.promptArgumentSuggestions = {
        ...this.promptArgumentSuggestions,
        [`${promptName}:${argumentName}`]: response.completion.completion.values
      };
      return response.completion.completion.values;
    } catch (error) {
      this.error = errorMessage(error, "Failed to fetch MCP argument completions");
      return [];
    } finally {
      this.busyAction = null;
    }
  }

  async completeResourceArgument(
    uriTemplate: string,
    argumentName: string,
    value: string,
    contextArguments: Record<string, string>
  ) {
    const serverName = this.selectedServerName;
    if (!serverName) return [];
    this.busyAction = `resource-complete:${uriTemplate}:${argumentName}`;
    this.error = null;
    try {
      const response = await apiClient.completeMcpArgument(serverName, {
        reference_type: "resource",
        reference_name: uriTemplate,
        argument_name: argumentName,
        value,
        context_arguments: contextArguments
      });
      return response.completion.completion.values;
    } catch (error) {
      this.error = errorMessage(error, "Failed to fetch MCP resource argument completions");
      return [];
    } finally {
      this.busyAction = null;
    }
  }

  async setRoots(roots: McpRootGrant[]) {
    const serverName = this.selectedServerName;
    if (!serverName) return;
    this.busyAction = "roots";
    this.error = null;
    try {
      const response = await apiClient.setMcpRoots(serverName, { roots });
      this.roots = response.roots;
      this.status = `Updated ${response.roots.length} root grants`;
      await this.fetchOverview();
    } catch (error) {
      this.error = errorMessage(error, "Failed to update MCP roots");
    } finally {
      this.busyAction = null;
    }
  }

  async toggleResourceSubscription(uri: string, subscribed: boolean) {
    const serverName = this.selectedServerName;
    if (!serverName) return;
    this.busyAction = `${subscribed ? "unsubscribe" : "subscribe"}:${uri}`;
    this.error = null;
    try {
      if (subscribed) {
        await apiClient.unsubscribeMcpResource(serverName, uri);
        this.status = `Unsubscribed from ${uri}`;
      } else {
        await apiClient.subscribeMcpResource(serverName, uri);
        this.status = `Subscribed to ${uri}`;
      }
      await this.refreshSelectedServer();
    } catch (error) {
      this.error = errorMessage(error, "Failed to update MCP resource subscription");
    } finally {
      this.busyAction = null;
    }
  }

  async respondSampling(
    taskId: string,
    payload: {
      action: "generate" | "approve" | "decline" | "cancel";
      request?: McpSamplingRequest | null;
      generated_text?: string | null;
    }
  ) {
    this.busyAction = `sampling:${taskId}:${payload.action}`;
    this.error = null;
    try {
      const response = await apiClient.respondMcpSampling(taskId, payload);
      this.status = `Sampling request ${payload.action}d`;
      await this.refreshSelectedServer();
      return response.task;
    } catch (error) {
      this.error = errorMessage(error, "Failed to handle MCP sampling request");
      return null;
    } finally {
      this.busyAction = null;
    }
  }

  async respondElicitation(
    taskId: string,
    payload: {
      action: "accept" | "decline" | "cancel";
      content?: Record<string, unknown> | null;
    }
  ) {
    this.busyAction = `elicitation:${taskId}:${payload.action}`;
    this.error = null;
    try {
      const response = await apiClient.respondMcpElicitation(taskId, payload);
      this.status = `Elicitation request ${payload.action}ed`;
      await this.refreshSelectedServer();
      return response.task;
    } catch (error) {
      this.error = errorMessage(error, "Failed to handle MCP elicitation request");
      return null;
    } finally {
      this.busyAction = null;
    }
  }
}

export const mcpStore = new McpState();
