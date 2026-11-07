import { describe, expect, test, vi, beforeEach } from "vitest";
import { render, screen } from "../utils/test-utils";
import { MCPConnectionStatus } from "../components/IntegrationsView/MCPServerView/MCPConnectionStatus";
import { MCPToolsList } from "../components/IntegrationsView/MCPServerView/MCPToolsList";
import badgeStyles from "../components/ui/Badge/Badge.module.css";
import type { MCPToolInfo } from "../services/refact/mcpServerInfo";

describe("MCPConnectionStatus", () => {
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

  test("string reconnecting shows yellow badge and spinner", () => {
    render(
      <MCPConnectionStatus
        status="reconnecting"
        onReconnect={vi.fn()}
        isReconnecting={false}
      />,
    );
    const badge = screen.getByLabelText("MCP connection status: reconnecting");
    expect(badge.classList.contains(badgeStyles.badge)).toBe(true);
    expect(badge.classList.contains(badgeStyles.warning)).toBe(true);
    expect(screen.getByRole("status", { name: "Reconnecting" })).toBeTruthy();
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

  test("renders destructive badge for destructive tools", () => {
    render(<MCPToolsList tools={tools} />);
    expect(screen.getByText("destructive")).toBeInTheDocument();
  });

  test("expands schema when show schema clicked", async () => {
    const { user } = render(<MCPToolsList tools={[tools[0]]} />);
    await user.click(screen.getByText("Show schema"));
    expect(screen.getByText("Hide schema")).toBeInTheDocument();
    expect(screen.getByText(/"type":/)).toBeInTheDocument();
  });
});

beforeEach(() => {
  vi.clearAllMocks();
});
