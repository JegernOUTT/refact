import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "../utils/test-utils";
import { http, HttpResponse } from "msw";
import { server } from "../utils/mockServer";
import { MCPOAuth } from "../components/IntegrationsView/MCPServerView/MCPOAuth";
import { MCPServerView } from "../components/IntegrationsView/MCPServerView";

const openSpy = vi.spyOn(window, "open").mockImplementation(() => null);

const CONFIG_PATH =
  "/home/user/.config/refact/integrations.d/mcp_http_myserver.yaml";

const PRELOADED_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

function mockStatus(body: object) {
  server.use(
    http.get("*/v1/mcp/oauth/status", () => {
      return HttpResponse.json({
        needs_login: false,
        oauth_available: false,
        suggested_scopes: [],
        expires_at: 0,
        scopes: [],
        ...body,
      });
    }),
  );
}

describe("MCPOAuth", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    openSpy.mockImplementation(() => null);
  });

  afterEach(() => {
    openSpy.mockImplementation(() => null);
  });

  test("renders nothing when auth_type is not oauth2_pkce", async () => {
    mockStatus({ auth_type: "bearer", authenticated: false });

    render(<MCPOAuth configPath={CONFIG_PATH} />, {
      preloadedState: PRELOADED_STATE,
    });

    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(
      screen.queryByRole("button", { name: /Login with OAuth/i }),
    ).toBeNull();
    expect(screen.queryByText("Authenticated")).toBeNull();
    expect(screen.queryByText("Not authenticated")).toBeNull();
  });

  test("shows login prompt when auth_type is none but server needs login and oauth is available", async () => {
    mockStatus({
      auth_type: "none",
      authenticated: false,
      needs_login: true,
      oauth_available: true,
    });

    render(<MCPOAuth configPath={CONFIG_PATH} />, {
      preloadedState: PRELOADED_STATE,
    });

    await waitFor(() => {
      expect(screen.getByText(/requires authentication/i)).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: /Login with OAuth/i }),
      ).toBeInTheDocument();
    });
  });

  test("keeps polling while the server discovers OAuth", async () => {
    let statusRequests = 0;
    server.use(
      http.get("*/v1/mcp/oauth/status", () => {
        statusRequests += 1;
        if (statusRequests === 1) {
          return HttpResponse.json({
            auth_type: "none",
            authenticated: false,
            needs_login: false,
            oauth_available: false,
            suggested_scopes: [],
            expires_at: 0,
            scopes: [],
          });
        }
        return HttpResponse.json({
          auth_type: "none",
          authenticated: false,
          needs_login: true,
          oauth_available: true,
          suggested_scopes: ["mcp.read"],
          expires_at: 0,
          scopes: [],
        });
      }),
    );

    render(
      <MCPOAuth
        configPath={CONFIG_PATH}
        connectionStatus={{ status: "connecting" }}
        authStatus="not_applicable"
        pollingIntervalMs={10}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    await waitFor(
      () => {
        expect(statusRequests).toBeGreaterThanOrEqual(2);
        expect(
          screen.getByRole("button", { name: /Login with OAuth/i }),
        ).toBeInTheDocument();
      },
      { timeout: 1000 },
    );
  });

  test("stops polling after authentication while the server reconnects", async () => {
    let statusRequests = 0;
    server.use(
      http.get("*/v1/mcp/oauth/status", () => {
        statusRequests += 1;
        return HttpResponse.json({
          auth_type: "oauth2_pkce",
          authenticated: true,
          needs_login: false,
          oauth_available: true,
          suggested_scopes: [],
          expires_at: Date.now() + 3600000,
          scopes: [],
        });
      }),
    );

    render(
      <MCPOAuth
        configPath={CONFIG_PATH}
        connectionStatus={{ status: "reconnecting" }}
        authStatus="needs_login"
        pollingIntervalMs={10}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    await waitFor(() => {
      expect(screen.getByText("Authenticated")).toBeInTheDocument();
    });
    const requestsAfterAuthentication = statusRequests;
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(statusRequests).toBe(requestsAfterAuthentication);
  });

  test("stops polling after a terminal non-OAuth failure", async () => {
    let statusRequests = 0;
    server.use(
      http.get("*/v1/mcp/oauth/status", () => {
        statusRequests += 1;
        return HttpResponse.json({
          auth_type: "none",
          authenticated: false,
          needs_login: true,
          oauth_available: false,
          suggested_scopes: [],
          expires_at: 0,
          scopes: [],
        });
      }),
    );

    render(
      <MCPOAuth
        configPath={CONFIG_PATH}
        connectionStatus={{ status: "needs_auth" }}
        authStatus="needs_login"
        pollingIntervalMs={10}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    await waitFor(() => expect(statusRequests).toBeGreaterThanOrEqual(1));
    const requestsAfterFailure = statusRequests;
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(statusRequests).toBe(requestsAfterFailure);
  });

  test("MCPServerView forwards live auth state until OAuth login appears", async () => {
    let oauthStatusRequests = 0;
    server.use(
      http.get("*/v1/mcp-server-info", () => {
        return HttpResponse.json({
          config_path: CONFIG_PATH,
          status: { status: "connecting" },
          auth_status: "not_applicable",
          tools: [],
          resources: [],
          prompts: [],
          capabilities: {
            tools: false,
            resources: false,
            prompts: false,
            sampling: true,
          },
          logs_tail: [],
          metrics: {},
          active_progress: [],
        });
      }),
      http.get("*/v1/mcp/oauth/status", () => {
        oauthStatusRequests += 1;
        if (oauthStatusRequests === 1) {
          return HttpResponse.json({
            auth_type: "none",
            authenticated: false,
            needs_login: false,
            oauth_available: false,
            suggested_scopes: [],
            expires_at: 0,
            scopes: [],
          });
        }
        return HttpResponse.json({
          auth_type: "none",
          authenticated: false,
          needs_login: true,
          oauth_available: true,
          suggested_scopes: ["mcp.read"],
          expires_at: 0,
          scopes: [],
        });
      }),
      http.post("*/v1/integration-get", () => {
        return HttpResponse.json({
          project_path: "",
          integr_name: "mcp_http_test",
          integr_config_path: CONFIG_PATH,
          integr_schema: {
            fields: {},
            available: {
              on_your_laptop: true,
              when_isolated: true,
            },
            confirmation: {
              ask_user_default: [],
              deny_default: [],
            },
          },
          integr_values: { url: "https://mcp.example.com" },
          error_log: [],
        });
      }),
      http.post("*/v1/integrations-mcp-logs", () => {
        return HttpResponse.json({ logs: [] });
      }),
    );

    render(
      <MCPServerView configPath={CONFIG_PATH} integrName="mcp_http_test" />,
      {
        preloadedState: PRELOADED_STATE,
      },
    );

    await waitFor(
      () => {
        expect(oauthStatusRequests).toBeGreaterThanOrEqual(2);
        expect(
          screen.getByRole("button", { name: /Login with OAuth/i }),
        ).toBeInTheDocument();
      },
      { timeout: 5000 },
    );
  });

  test("renders nothing when server needs login but oauth is not available", async () => {
    mockStatus({
      auth_type: "none",
      authenticated: false,
      needs_login: true,
      oauth_available: false,
    });

    render(<MCPOAuth configPath={CONFIG_PATH} />, {
      preloadedState: PRELOADED_STATE,
    });

    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(
      screen.queryByRole("button", { name: /Login with OAuth/i }),
    ).toBeNull();
    expect(screen.queryByText(/requires authentication/i)).toBeNull();
    expect(screen.queryByText("Not authenticated")).toBeNull();
  });

  test("login click starts oauth flow when auth_type is none", async () => {
    mockStatus({
      auth_type: "none",
      authenticated: false,
      needs_login: true,
      oauth_available: true,
    });
    server.use(
      http.post("*/v1/mcp/oauth/start", () => {
        return HttpResponse.json({
          session_id: "s1",
          authorize_url: "https://auth.example.com/authorize?state=x",
        });
      }),
    );

    const { user } = render(<MCPOAuth configPath={CONFIG_PATH} />, {
      preloadedState: PRELOADED_STATE,
    });

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /Login with OAuth/i }),
      ).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: /Login with OAuth/i }));

    await waitFor(() => {
      expect(
        screen.getByText("Waiting for authorization..."),
      ).toBeInTheDocument();
    });
  });

  test("renders Login button when not authenticated", async () => {
    mockStatus({ auth_type: "oauth2_pkce", authenticated: false });

    render(<MCPOAuth configPath={CONFIG_PATH} />, {
      preloadedState: PRELOADED_STATE,
    });

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /Login with OAuth/i }),
      ).toBeInTheDocument();
    });
  });

  test("shows not authenticated badge when auth_type is oauth2_pkce and not authenticated", async () => {
    mockStatus({ auth_type: "oauth2_pkce", authenticated: false });

    render(<MCPOAuth configPath={CONFIG_PATH} />, {
      preloadedState: PRELOADED_STATE,
    });

    await waitFor(() => {
      expect(screen.getByText("Not authenticated")).toBeInTheDocument();
    });
  });

  test("shows waiting state after login click", async () => {
    mockStatus({ auth_type: "oauth2_pkce", authenticated: false });
    server.use(
      http.post("*/v1/mcp/oauth/start", () => {
        return HttpResponse.json({
          session_id: "test-session-123",
          authorize_url:
            "https://auth.example.com/authorize?code_challenge=abc",
        });
      }),
    );

    const { user } = render(<MCPOAuth configPath={CONFIG_PATH} />, {
      preloadedState: PRELOADED_STATE,
    });

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /Login with OAuth/i }),
      ).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: /Login with OAuth/i }));

    await waitFor(() => {
      expect(
        screen.getByText("Waiting for authorization..."),
      ).toBeInTheDocument();
    });
  });

  test("shows authenticated state with logout button", async () => {
    mockStatus({
      auth_type: "oauth2_pkce",
      authenticated: true,
      expires_at: Date.now() + 3600000,
    });

    render(<MCPOAuth configPath={CONFIG_PATH} />, {
      preloadedState: PRELOADED_STATE,
    });

    await waitFor(() => {
      expect(screen.getByText("Authenticated")).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: /Logout/i }),
      ).toBeInTheDocument();
    });
  });

  test("shows session expired badge when expires_at is in the past", async () => {
    mockStatus({
      auth_type: "oauth2_pkce",
      authenticated: false,
      expires_at: Date.now() - 10000,
    });

    render(<MCPOAuth configPath={CONFIG_PATH} />, {
      preloadedState: PRELOADED_STATE,
    });

    await waitFor(() => {
      expect(screen.getByText("Session expired")).toBeInTheDocument();
      expect(
        screen.getByText(/Session expired, please re-login/i),
      ).toBeInTheDocument();
    });
  });

  test("manual code entry shows Submit Code button in waiting state", async () => {
    mockStatus({ auth_type: "oauth2_pkce", authenticated: false });
    server.use(
      http.post("*/v1/mcp/oauth/start", () => {
        return HttpResponse.json({
          session_id: "test-session-456",
          authorize_url: "https://auth.example.com/authorize",
        });
      }),
    );

    const { user } = render(<MCPOAuth configPath={CONFIG_PATH} />, {
      preloadedState: PRELOADED_STATE,
    });

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /Login with OAuth/i }),
      ).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: /Login with OAuth/i }));

    await waitFor(() => {
      expect(screen.getByLabelText("Authorization code")).toBeInTheDocument();
    });

    const codeInput = screen.getByLabelText("Authorization code");
    fireEvent.change(codeInput, { target: { value: "test-auth-code" } });

    expect(
      screen.getByRole("button", { name: /Submit Code/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /Submit Code/i }),
    ).not.toBeDisabled();
  });

  test("logout calls logout endpoint", async () => {
    let logoutCalled = false;

    mockStatus({
      auth_type: "oauth2_pkce",
      authenticated: true,
    });
    server.use(
      http.post("*/v1/mcp/oauth/logout", () => {
        logoutCalled = true;
        return HttpResponse.json({ success: true });
      }),
    );

    const { user } = render(<MCPOAuth configPath={CONFIG_PATH} />, {
      preloadedState: PRELOADED_STATE,
    });

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /Logout/i }),
      ).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: /Logout/i }));

    await waitFor(() => {
      expect(logoutCalled).toBe(true);
    });
  });

  test("shows error message on failed login start", async () => {
    mockStatus({ auth_type: "oauth2_pkce", authenticated: false });
    server.use(
      http.post("*/v1/mcp/oauth/start", () => {
        return HttpResponse.json(
          { detail: "Server unreachable" },
          { status: 500 },
        );
      }),
    );

    const { user } = render(<MCPOAuth configPath={CONFIG_PATH} />, {
      preloadedState: PRELOADED_STATE,
    });

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /Login with OAuth/i }),
      ).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: /Login with OAuth/i }));

    await waitFor(() => {
      expect(screen.getByText(/Failed to start OAuth/i)).toBeInTheDocument();
    });
  });

  test("cancel button shown during waiting state", async () => {
    mockStatus({ auth_type: "oauth2_pkce", authenticated: false });
    server.use(
      http.post("*/v1/mcp/oauth/start", () => {
        return HttpResponse.json({
          session_id: "test-session-cancel-show",
          authorize_url: "https://auth.example.com/authorize",
        });
      }),
    );

    const { user } = render(<MCPOAuth configPath={CONFIG_PATH} />, {
      preloadedState: PRELOADED_STATE,
    });

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /Login with OAuth/i }),
      ).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: /Login with OAuth/i }));

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /Cancel/i }),
      ).toBeInTheDocument();
    });
  });

  test("cancel calls backend with session_id", async () => {
    let cancelledSessionId: string | null = null;

    mockStatus({ auth_type: "oauth2_pkce", authenticated: false });
    server.use(
      http.post("*/v1/mcp/oauth/start", () => {
        return HttpResponse.json({
          session_id: "test-session-to-cancel",
          authorize_url: "https://auth.example.com/authorize",
        });
      }),
      http.post("*/v1/mcp/oauth/cancel", async ({ request }) => {
        const body = (await request.json()) as { session_id: string };
        cancelledSessionId = body.session_id;
        return HttpResponse.json({ cancelled: true });
      }),
    );

    const { user } = render(<MCPOAuth configPath={CONFIG_PATH} />, {
      preloadedState: PRELOADED_STATE,
    });

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /Login with OAuth/i }),
      ).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: /Login with OAuth/i }));

    await waitFor(() => {
      expect(
        screen.getByText("Waiting for authorization..."),
      ).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: /Cancel/i }));

    await waitFor(() => {
      expect(cancelledSessionId).toBe("test-session-to-cancel");
    });

    await waitFor(() => {
      expect(screen.getByText("Not authenticated")).toBeInTheDocument();
    });
  });

  test("polling stops when authenticated", async () => {
    let callCount = 0;

    server.use(
      http.get("*/v1/mcp/oauth/status", () => {
        callCount++;
        return HttpResponse.json({
          auth_type: "oauth2_pkce",
          authenticated: true,
          expires_at: Date.now() + 3600000,
          scopes: [],
          needs_login: false,
          oauth_available: false,
          suggested_scopes: [],
        });
      }),
    );

    render(<MCPOAuth configPath={CONFIG_PATH} />, {
      preloadedState: PRELOADED_STATE,
    });

    await waitFor(() => {
      expect(screen.getByText("Authenticated")).toBeInTheDocument();
    });

    const countAfterAuth = callCount;
    await new Promise((r) => setTimeout(r, 100));
    expect(callCount).toBe(countAfterAuth);
  });
});
