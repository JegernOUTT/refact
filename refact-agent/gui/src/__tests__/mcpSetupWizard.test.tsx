import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "../utils/test-utils";
import { http, HttpResponse } from "msw";
import { server } from "../utils/mockServer";
import { MCPSetupWizard } from "../components/IntegrationsView/MCPSetupWizard";
import type { NotConfiguredIntegrationWithIconRecord } from "../services/refact";

const MOCK_INTEGRATION: NotConfiguredIntegrationWithIconRecord = {
  integr_name: "mcp_TEMPLATE",
  integr_config_path: ["/home/user/.config/refact/integrations.d/mcp_TEMPLATE"],
  project_path: [""],
  icon_path: "/icons/mcp.svg",
  integr_config_exists: false,
  wasOpenedThroughChat: false,
  when_isolated: false,
  on_your_laptop: false,
};

const PRELOADED_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

describe("MCPSetupWizard", () => {
  it("typing a command shows Local server (stdio) detection", () => {
    render(
      <MCPSetupWizard
        integration={MOCK_INTEGRATION}
        onSubmit={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    const input = screen.getByTestId("mcp-wizard-input");
    fireEvent.change(input, {
      target: { value: "npx -y @notionhq/notion-mcp-server" },
    });

    expect(screen.getByText(/Local server \(stdio\)/)).toBeDefined();
  });

  it("typing a URL shows Remote server (HTTP) detection", () => {
    render(
      <MCPSetupWizard
        integration={MOCK_INTEGRATION}
        onSubmit={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    const input = screen.getByTestId("mcp-wizard-input");
    fireEvent.change(input, {
      target: { value: "https://api.example.com/mcp" },
    });

    expect(screen.getByText(/Remote server \(HTTP\)/)).toBeDefined();
  });

  it("name auto-populated from auto-name API response", async () => {
    server.use(
      http.post("*/v1/mcp/auto-name", () => {
        return HttpResponse.json({
          suggested_name: "notion_mcp_server",
          transport: "stdio",
          config_prefix: "mcp_stdio_",
        });
      }),
    );

    render(
      <MCPSetupWizard
        integration={MOCK_INTEGRATION}
        onSubmit={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    const input = screen.getByTestId("mcp-wizard-input");
    fireEvent.change(input, {
      target: { value: "npx -y @notionhq/notion-mcp-server" },
    });

    await waitFor(
      () => {
        const nameField = screen.getByTestId("mcp-wizard-name");
        expect((nameField as HTMLInputElement).value).toBe("notion_mcp_server");
      },
      { timeout: 2000 },
    );
  });

  it("name validation rejects invalid snake_case", async () => {
    server.use(
      http.post("*/v1/mcp/auto-name", () => {
        return HttpResponse.json({
          suggested_name: "notion_mcp_server",
          transport: "stdio",
          config_prefix: "mcp_stdio_",
        });
      }),
    );

    render(
      <MCPSetupWizard
        integration={MOCK_INTEGRATION}
        onSubmit={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    const input = screen.getByTestId("mcp-wizard-input");
    fireEvent.change(input, { target: { value: "npx test" } });

    const nameField = await screen.findByTestId("mcp-wizard-name");
    fireEvent.change(nameField, { target: { value: "Invalid Name!" } });

    expect(screen.getByText(/snake_case/i)).toBeDefined();
  });

  it("Continue with setup creates correct config path for stdio command", async () => {
    const calls: {
      configPath: string;
      integrName: string;
      initialInput?: { input: string; transport: string };
    }[] = [];

    server.use(
      http.post("*/v1/mcp/auto-name", () => {
        return HttpResponse.json({
          suggested_name: "notion_server",
          transport: "stdio",
          config_prefix: "mcp_stdio_",
        });
      }),
    );

    render(
      <MCPSetupWizard
        integration={MOCK_INTEGRATION}
        onSubmit={(configPath, integrName, initialInput) => {
          calls.push({ configPath, integrName, initialInput });
        }}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    const input = screen.getByTestId("mcp-wizard-input");
    fireEvent.change(input, { target: { value: "npx notion" } });

    const nameField = await screen.findByTestId("mcp-wizard-name");
    fireEvent.change(nameField, { target: { value: "notion_server" } });

    const submitBtn = screen.getByTestId("mcp-wizard-submit");
    fireEvent.click(submitBtn);

    expect(calls.length).toBe(1);
    expect(calls[0]?.integrName).toBe("mcp_stdio_notion_server");
    expect(calls[0]?.configPath).toContain("mcp_stdio_notion_server");
    expect(calls[0]?.initialInput?.input).toBe("npx notion");
    expect(calls[0]?.initialInput?.transport).toBe("stdio");
  });

  it("Continue with setup passes initialInput with http transport for URL inputs", async () => {
    const calls: {
      configPath: string;
      integrName: string;
      initialInput?: { input: string; transport: string };
    }[] = [];

    server.use(
      http.post("*/v1/mcp/auto-name", () => {
        return HttpResponse.json({
          suggested_name: "example_mcp",
          transport: "http",
          config_prefix: "mcp_http_",
        });
      }),
    );

    render(
      <MCPSetupWizard
        integration={MOCK_INTEGRATION}
        onSubmit={(configPath, integrName, initialInput) => {
          calls.push({ configPath, integrName, initialInput });
        }}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    const input = screen.getByTestId("mcp-wizard-input");
    fireEvent.change(input, {
      target: { value: "https://api.example.com/mcp" },
    });

    const nameField = await screen.findByTestId("mcp-wizard-name");
    fireEvent.change(nameField, { target: { value: "example_mcp" } });

    const submitBtn = screen.getByTestId("mcp-wizard-submit");
    fireEvent.click(submitBtn);

    expect(calls.length).toBe(1);
    expect(calls[0]?.initialInput?.input).toBe("https://api.example.com/mcp");
    expect(calls[0]?.initialInput?.transport).toBe("http");
  });

  it("fallback name used when auto-name API unavailable", async () => {
    server.use(
      http.post("*/v1/mcp/auto-name", () => {
        return HttpResponse.error();
      }),
    );

    render(
      <MCPSetupWizard
        integration={MOCK_INTEGRATION}
        onSubmit={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    const input = screen.getByTestId("mcp-wizard-input");
    fireEvent.change(input, { target: { value: "npx my-server" } });

    await waitFor(
      () => {
        const nameField = screen.getByTestId("mcp-wizard-name");
        expect((nameField as HTMLInputElement).value).toBeTruthy();
      },
      { timeout: 2000 },
    );
  });
});

describe("MCPSetupWizard - SSE advanced toggle", () => {
  it("shows SSE checkbox under Advanced for stdio commands", () => {
    render(
      <MCPSetupWizard integration={MOCK_INTEGRATION} onSubmit={vi.fn()} />,
      { preloadedState: PRELOADED_STATE },
    );

    const input = screen.getByTestId("mcp-wizard-input");
    fireEvent.change(input, { target: { value: "npx some-server" } });

    const advancedBtn = screen.getByText(/Advanced: Use SSE transport/i);
    fireEvent.click(advancedBtn);

    expect(screen.getByTestId("mcp-wizard-sse-checkbox")).toBeDefined();
  });

  it("does not show SSE checkbox for URL inputs", () => {
    render(
      <MCPSetupWizard integration={MOCK_INTEGRATION} onSubmit={vi.fn()} />,
      { preloadedState: PRELOADED_STATE },
    );

    const input = screen.getByTestId("mcp-wizard-input");
    fireEvent.change(input, {
      target: { value: "https://api.example.com/mcp" },
    });

    expect(screen.queryByText(/Advanced: Use SSE transport/i)).toBeNull();
  });
});
