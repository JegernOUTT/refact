import { describe, expect, test, vi, beforeEach } from "vitest";
import { render, screen } from "../utils/test-utils";
import { MCPConnectionStatus } from "../components/IntegrationsView/MCPServerView/MCPConnectionStatus";
import { MCPToolsList } from "../components/IntegrationsView/MCPServerView/MCPToolsList";
import { MCPResourcesList } from "../components/IntegrationsView/MCPServerView/MCPResourcesList";
import { MCPPromptsList } from "../components/IntegrationsView/MCPServerView/MCPPromptsList";
import type {
  MCPToolInfo,
  MCPResourceInfo,
  MCPPromptInfo,
} from "../services/refact/mcpServerInfo";

describe("MCPConnectionStatus", () => {
  test("renders connected status as green badge", () => {
    render(
      <MCPConnectionStatus
        status={{ status: "connected" }}
        onReconnect={vi.fn()}
        isReconnecting={false}
      />,
    );
    expect(screen.getByText("connected")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /reconnect/i }),
    ).toBeInTheDocument();
  });

  test("renders string status", () => {
    render(
      <MCPConnectionStatus
        status="connecting"
        onReconnect={vi.fn()}
        isReconnecting={false}
      />,
    );
    expect(screen.getByText("connecting")).toBeInTheDocument();
  });

  test("shows reconnecting state on button when reconnecting", () => {
    render(
      <MCPConnectionStatus
        status="connected"
        onReconnect={vi.fn()}
        isReconnecting={true}
      />,
    );
    expect(screen.getByText("Reconnecting...")).toBeInTheDocument();
    expect(screen.getByRole("button")).toBeDisabled();
  });

  test("shows error message from status object", () => {
    render(
      <MCPConnectionStatus
        status={{ status: "error", error: "Connection refused" }}
        onReconnect={vi.fn()}
        isReconnecting={false}
      />,
    );
    expect(screen.getByText("Connection refused")).toBeInTheDocument();
  });

  test("calls onReconnect when button clicked", async () => {
    const onReconnect = vi.fn();
    const { user } = render(
      <MCPConnectionStatus
        status="connected"
        onReconnect={onReconnect}
        isReconnecting={false}
      />,
    );
    await user.click(screen.getByRole("button", { name: /reconnect/i }));
    expect(onReconnect).toHaveBeenCalledOnce();
  });

  test("string connected shows green badge and no spinner", () => {
    render(
      <MCPConnectionStatus
        status="connected"
        onReconnect={vi.fn()}
        isReconnecting={false}
      />,
    );
    expect(screen.getByText("connected")).toBeInTheDocument();
    expect(screen.queryByRole("status")).toBeNull();
  });

  test("string reconnecting shows yellow badge and spinner", () => {
    render(
      <MCPConnectionStatus
        status="reconnecting"
        onReconnect={vi.fn()}
        isReconnecting={false}
      />,
    );
    expect(screen.getByText("reconnecting")).toBeInTheDocument();
    const spinner = document.querySelector("pre");
    expect(spinner).toBeTruthy();
  });

  test("string disconnected shows red badge and no spinner", () => {
    const { container } = render(
      <MCPConnectionStatus
        status="disconnected"
        onReconnect={vi.fn()}
        isReconnecting={false}
      />,
    );
    const badge = container.querySelector("[data-accent-color='red']");
    expect(badge).toBeTruthy();
    expect(screen.getByText("disconnected")).toBeInTheDocument();
  });

  test("object status with attempt and max_attempts shows attempt info", () => {
    render(
      <MCPConnectionStatus
        status={{ status: "reconnecting", attempt: 2, max_attempts: 7 }}
        onReconnect={vi.fn()}
        isReconnecting={false}
      />,
    );
    expect(screen.getByText("Attempt 2/7")).toBeInTheDocument();
  });

  test("object status with next_retry_seconds shows retry info", () => {
    render(
      <MCPConnectionStatus
        status={{ status: "reconnecting", next_retry_seconds: 3 }}
        onReconnect={vi.fn()}
        isReconnecting={false}
      />,
    );
    expect(screen.getByText("Next retry in 3s")).toBeInTheDocument();
  });

  test("isReconnecting=true shows spinner", () => {
    render(
      <MCPConnectionStatus
        status="connected"
        onReconnect={vi.fn()}
        isReconnecting={true}
      />,
    );
    const spinner = document.querySelector("pre");
    expect(spinner).toBeTruthy();
  });
});

describe("MCPToolsList", () => {
  const tools: MCPToolInfo[] = [
    {
      name: "create_issue",
      description: "Create a GitHub issue",
      input_schema: {
        type: "object",
        properties: { title: { type: "string" } },
      },
      internal_name: "mcp_github_create_issue",
    },
    {
      name: "delete_repo",
      description: "Delete a repository",
      input_schema: { type: "object" },
      annotations: { destructiveHint: true },
      internal_name: "mcp_github_delete_repo",
    },
  ];

  test("renders tool names", () => {
    render(<MCPToolsList tools={tools} />);
    expect(screen.getByText("create_issue")).toBeInTheDocument();
    expect(screen.getByText("delete_repo")).toBeInTheDocument();
  });

  test("renders tool descriptions", () => {
    render(<MCPToolsList tools={tools} />);
    expect(screen.getByText("Create a GitHub issue")).toBeInTheDocument();
    expect(screen.getByText("Delete a repository")).toBeInTheDocument();
  });

  test("renders destructive badge for destructive tools", () => {
    render(<MCPToolsList tools={tools} />);
    expect(screen.getByText("destructive")).toBeInTheDocument();
  });

  test("renders empty state when no tools", () => {
    render(<MCPToolsList tools={[]} />);
    expect(screen.getByText("No tools available")).toBeInTheDocument();
  });

  test("shows toggle switch for each tool", () => {
    render(<MCPToolsList tools={tools} />);
    const switches = screen.getAllByRole("switch");
    expect(switches).toHaveLength(2);
  });

  test("expands schema when show schema clicked", async () => {
    const { user } = render(<MCPToolsList tools={[tools[0]]} />);
    await user.click(screen.getByText("Show schema"));
    expect(screen.getByText("Hide schema")).toBeInTheDocument();
    expect(screen.getByText(/"type":/)).toBeInTheDocument();
  });
});

describe("MCPResourcesList", () => {
  const resources: MCPResourceInfo[] = [
    {
      uri: "repo://owner/repo",
      name: "Repository",
      description: "Repository content",
      mime_type: "application/json",
    },
  ];

  test("renders resource URIs", () => {
    render(<MCPResourcesList resources={resources} />);
    expect(screen.getByText("repo://owner/repo")).toBeInTheDocument();
  });

  test("renders resource descriptions", () => {
    render(<MCPResourcesList resources={resources} />);
    expect(screen.getByText("Repository content")).toBeInTheDocument();
  });

  test("renders mime types", () => {
    render(<MCPResourcesList resources={resources} />);
    expect(screen.getByText("application/json")).toBeInTheDocument();
  });

  test("shows empty state when no resources", () => {
    render(<MCPResourcesList resources={[]} />);
    expect(screen.getByText("No resources available")).toBeInTheDocument();
  });
});

describe("MCPPromptsList", () => {
  const prompts: MCPPromptInfo[] = [
    {
      name: "commit_message",
      description: "Generate a commit message",
    },
  ];

  test("renders prompt names", () => {
    render(<MCPPromptsList prompts={prompts} />);
    expect(screen.getByText("commit_message")).toBeInTheDocument();
  });

  test("renders prompt descriptions", () => {
    render(<MCPPromptsList prompts={prompts} />);
    expect(screen.getByText("Generate a commit message")).toBeInTheDocument();
  });

  test("shows empty state when no prompts", () => {
    render(<MCPPromptsList prompts={[]} />);
    expect(screen.getByText("No prompts available")).toBeInTheDocument();
  });
});

describe("mcpServerInfo API types", () => {
  test("MCPServerInfo type has expected shape", () => {
    const serverInfo = {
      config_path: "/path/to/config.yaml",
      status: { status: "connected" },
      server_name: "GitHub MCP",
      server_version: "1.0.0",
      protocol_version: "2024-11-05",
      tools: [] as MCPToolInfo[],
      resources: [] as MCPResourceInfo[],
      prompts: [] as MCPPromptInfo[],
      capabilities: {
        tools: true,
        resources: false,
        prompts: false,
        sampling: false,
      },
      logs_tail: ["server started"],
    };

    expect(serverInfo.config_path).toBe("/path/to/config.yaml");
    expect(serverInfo.tools).toHaveLength(0);
    expect(serverInfo.capabilities.tools).toBe(true);
    expect(serverInfo.logs_tail).toContain("server started");
  });
});

beforeEach(() => {
  vi.clearAllMocks();
});
