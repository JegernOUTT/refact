import { describe, expect, test } from "vitest";
import { render, screen, waitFor } from "../utils/test-utils";
import { http, HttpResponse } from "msw";
import { server } from "../utils/mockServer";
import { MCPToolsList } from "../components/IntegrationsView/MCPServerView/MCPToolsList";
import type { MCPToolInfo } from "../services/refact/mcpServerInfo";

const CONFIG_PATH =
  "/home/user/.config/refact/integrations.d/mcp_stdio_test.yaml";

const PRELOADED_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

const TOOLS: MCPToolInfo[] = [
  {
    name: "Tool One",
    description: "First description",
    input_schema: { type: "object" },
    internal_name: "internal_tool_one",
  },
  {
    name: "Tool Two",
    description: "Second description",
    input_schema: { type: "object" },
    internal_name: "internal_tool_two",
  },
];

const baseValues = {
  command: "run-server",
  available: { on_your_laptop: true, when_isolated: true },
  keep_me: "unchanged",
};

function integrationResponse(
  integrValues: Record<string, unknown> = {},
): Record<string, unknown> {
  return {
    project_path: "/home/user/project",
    integr_name: "mcp_test",
    integr_config_path: CONFIG_PATH,
    integr_schema: {
      fields: {},
      available: { on_your_laptop: true, when_isolated: true },
      confirmation: { ask_user_default: [], deny_default: [] },
    },
    integr_values: {
      ...baseValues,
      ...integrValues,
    },
    error_log: [],
  };
}

function mockIntegration(values: Record<string, unknown>) {
  server.use(
    http.post("*/v1/integration-get", () => {
      return HttpResponse.json(integrationResponse(values));
    }),
  );
}

function renderList() {
  return render(<MCPToolsList tools={TOOLS} configPath={CONFIG_PATH} />, {
    preloadedState: PRELOADED_STATE,
  });
}

describe("MCPToolsList tool toggles", () => {
  test("tools render with both switches and disabled_tools turns Enabled off with dimmed row content", async () => {
    mockIntegration({ disabled_tools: "Tool Two" });

    renderList();

    expect(
      await screen.findByRole("switch", { name: "Tool One Enabled" }),
    ).toBeChecked();
    expect(
      screen.getByRole("switch", { name: "Tool One Auto-approve" }),
    ).toBeInTheDocument();

    await waitFor(() => {
      expect(
        screen.getByRole("switch", { name: "Tool Two Enabled" }),
      ).not.toBeChecked();
    });
    expect(
      screen.getByRole("switch", { name: "Tool Two Auto-approve" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Second description").className).toContain(
      "toolContentDisabled",
    );
  });

  test("toggling Enabled off posts save with updated comma-separated disabled_tools", async () => {
    let savedBody: unknown;
    mockIntegration({
      disabled_tools: "Tool Two",
      auto_approve_tools: "Tool Two",
    });
    server.use(
      http.post("*/v1/integration-save", async ({ request }) => {
        savedBody = await request.json();
        return HttpResponse.json({});
      }),
    );

    const { user } = renderList();

    const enabledSwitch = await screen.findByRole("switch", {
      name: "Tool One Enabled",
    });
    await user.click(enabledSwitch);

    await waitFor(() => {
      expect(savedBody).toMatchObject({
        integr_config_path: CONFIG_PATH,
        integr_values: {
          ...baseValues,
          disabled_tools: "Tool Two,Tool One",
          auto_approve_tools: "Tool Two",
        },
      });
    });
  });

  test("auto-approve toggle is disabled when tool is disabled", async () => {
    mockIntegration({ disabled_tools: ["Tool Two"] });

    renderList();

    await waitFor(() => {
      expect(
        screen.getByRole("switch", { name: "Tool Two Auto-approve" }),
      ).toBeDisabled();
    });
  });

  test("toggling Auto-approve on posts save with updated auto_approve_tools", async () => {
    let savedBody: unknown;
    mockIntegration({ disabled_tools: "", auto_approve_tools: "Tool Two" });
    server.use(
      http.post("*/v1/integration-save", async ({ request }) => {
        savedBody = await request.json();
        return HttpResponse.json({});
      }),
    );

    const { user } = renderList();

    const autoApproveSwitch = await screen.findByRole("switch", {
      name: "Tool One Auto-approve",
    });
    await user.click(autoApproveSwitch);

    await waitFor(() => {
      expect(savedBody).toMatchObject({
        integr_config_path: CONFIG_PATH,
        integr_values: {
          ...baseValues,
          disabled_tools: "",
          auto_approve_tools: "Tool Two,Tool One",
        },
      });
    });
  });
});
