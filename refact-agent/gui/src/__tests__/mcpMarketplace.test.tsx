import { describe, expect, it } from "vitest";
import { render, screen, fireEvent, waitFor } from "../utils/test-utils";
import { http, HttpResponse } from "msw";
import { server } from "../utils/mockServer";
import { setUpStore } from "../app/store";
import { MCPMarketplace } from "../features/MCPMarketplace";
import { ServerCard } from "../features/MCPMarketplace/ServerCard";
import { SourceSelector } from "../features/MCPMarketplace/SourceSelector";
import type {
  MCPServer,
  MarketplaceResponse,
  MarketplaceSource,
} from "../services/refact/mcpMarketplace";
import { mcpMarketplaceApi } from "../services/refact/mcpMarketplace";

const MOCK_SERVER: MCPServer = {
  id: "test-server",
  source_id: "refact-bundled",
  name: "Test Server",
  description: "A test MCP server for unit tests",
  publisher: "Test Publisher",
  tags: ["search", "code"],
  transport: "stdio",
  install_recipe: {
    command: "npx test-server",
    env: { API_KEY: "" },
  },
  confirmation_default: [],
};

// Same server but with every recipe env var pre-filled, so card-level install
// does not get routed to the detail view by the required-env gate.
const MOCK_SERVER_NO_REQUIRED_ENV: MCPServer = {
  ...MOCK_SERVER,
  install_recipe: {
    command: "npx test-server",
    env: { API_KEY: "prefilled-key" },
  },
};

const MOCK_SOURCES: MarketplaceSource[] = [
  {
    id: "refact-bundled",
    label: "Refact Built-in",
    type: "refact_index",
    enabled: true,
    removable: false,
    server_count: 1,
    status: "ok",
  },
  {
    id: "smithery",
    label: "Smithery.ai",
    type: "smithery",
    enabled: false,
    removable: false,
    server_count: 0,
    needs_api_key: true,
    has_api_key: false,
  },
  {
    id: "official-mcp",
    label: "MCP Registry",
    type: "official_mcp",
    enabled: true,
    removable: false,
    server_count: 50,
    status: "ok",
  },
];

const MOCK_RESPONSE: MarketplaceResponse = {
  servers: [MOCK_SERVER],
  sources: MOCK_SOURCES,
};

const PRELOADED_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

describe("mcpMarketplaceApi", () => {
  it("uses relative marketplace URLs for dev web configs", async () => {
    const marketplaceUrls: string[] = [];
    server.use(
      http.get("/v1/mcp/marketplace", ({ request }) => {
        const url = new URL(request.url);
        marketplaceUrls.push(`${url.pathname}${url.search}`);
        return HttpResponse.json(MOCK_RESPONSE);
      }),
    );

    const store = setUpStore({
      config: {
        apiKey: "test",
        lspPort: 8001,
        themeProps: {},
        host: "web",
        dev: true,
      },
    });

    try {
      await store
        .dispatch(
          mcpMarketplaceApi.endpoints.getMarketplace.initiate({
            q: "test server",
            page: 2,
          }),
        )
        .unwrap();

      expect(marketplaceUrls).toEqual([
        "/v1/mcp/marketplace?q=test+server&page=2",
      ]);
    } finally {
      store.dispatch(mcpMarketplaceApi.util.resetApiState());
    }
  });

  it("uses sanitized remote marketplace URLs for standalone web configs", async () => {
    let installUrl = "";
    server.use(
      http.post(
        "https://remote.example.test/refact/v1/mcp/marketplace/install",
        ({ request }) => {
          installUrl = request.url;
          return HttpResponse.json({
            installed: true,
            config_path: "/tmp/server.yaml",
          });
        },
      ),
    );

    const store = setUpStore({
      config: {
        apiKey: "test",
        lspPort: 8001,
        themeProps: {},
        host: "web",
        lspUrl: "https://remote.example.test/refact/v1/ping/Refact",
      },
    });

    try {
      await store
        .dispatch(
          mcpMarketplaceApi.endpoints.installServer.initiate({
            server_id: "test-server",
          }),
        )
        .unwrap();

      expect(installUrl).toBe(
        "https://remote.example.test/refact/v1/mcp/marketplace/install",
      );
    } finally {
      store.dispatch(mcpMarketplaceApi.util.resetApiState());
    }
  });
});

describe("ServerCard", () => {
  it("renders server name, publisher and description", () => {
    render(
      <ServerCard
        server={MOCK_SERVER}
        isInstalled={false}
        isInstalling={false}
        onInstall={() => undefined}
        onViewDetail={() => undefined}
      />,
    );
    expect(screen.getByText("Test Server")).toBeDefined();
    expect(screen.getByText("Test Publisher")).toBeDefined();
    expect(screen.getByText("A test MCP server for unit tests")).toBeDefined();
  });

  it("renders Install button when not installed", () => {
    render(
      <ServerCard
        server={MOCK_SERVER}
        isInstalled={false}
        isInstalling={false}
        onInstall={() => undefined}
        onViewDetail={() => undefined}
      />,
    );
    expect(screen.getByRole("button", { name: /install/i })).toBeDefined();
    expect(screen.queryByText("Installed")).toBeNull();
  });

  it("renders Installed text when installed", () => {
    render(
      <ServerCard
        server={MOCK_SERVER}
        isInstalled={true}
        isInstalling={false}
        onInstall={() => undefined}
        onViewDetail={() => undefined}
      />,
    );
    expect(screen.getByText("Installed")).toBeDefined();
    expect(screen.queryByRole("button", { name: /^install$/i })).toBeNull();
  });

  it("renders tags as badges", () => {
    render(
      <ServerCard
        server={MOCK_SERVER}
        isInstalled={false}
        isInstalling={false}
        onInstall={() => undefined}
        onViewDetail={() => undefined}
      />,
    );
    expect(screen.getByText("search")).toBeDefined();
    expect(screen.getByText("code")).toBeDefined();
  });

  it("calls onInstall with server when Install button clicked", () => {
    const calledWith: MCPServer[] = [];
    render(
      <ServerCard
        server={MOCK_SERVER}
        isInstalled={false}
        isInstalling={false}
        onInstall={(s) => {
          calledWith.push(s);
        }}
        onViewDetail={() => undefined}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /install/i }));
    expect(calledWith.length).toBe(1);
    expect(calledWith[0]?.id).toBe("test-server");
  });

  it("renders source badge when sourceLabel is provided", () => {
    render(
      <ServerCard
        server={MOCK_SERVER}
        isInstalled={false}
        isInstalling={false}
        onInstall={() => undefined}
        onViewDetail={() => undefined}
        sourceLabel="Refact Built-in"
      />,
    );
    expect(screen.getByText("Refact Built-in")).toBeDefined();
  });

  it("shows Update and Uninstall for installed servers with a newer recipe", () => {
    const updates: string[] = [];
    const uninstalls: string[] = [];
    render(
      <ServerCard
        server={MOCK_SERVER}
        isInstalled={true}
        installedConfigPath="/tmp/mcp_stdio_test_server.yaml"
        updateAvailable={true}
        isInstalling={false}
        onInstall={() => undefined}
        onViewDetail={() => undefined}
        onConfigure={() => undefined}
        onUpdate={(p) => updates.push(p)}
        onUninstall={(p) => uninstalls.push(p)}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /update/i }));
    expect(updates).toEqual(["/tmp/mcp_stdio_test_server.yaml"]);

    // Uninstall is a two-step confirm.
    const uninstallBtn = screen.getByRole("button", { name: /uninstall/i });
    fireEvent.click(uninstallBtn);
    expect(uninstalls).toEqual([]);
    fireEvent.click(screen.getByRole("button", { name: /confirm/i }));
    expect(uninstalls).toEqual(["/tmp/mcp_stdio_test_server.yaml"]);
  });

  it("hides Update when no newer recipe is available", () => {
    render(
      <ServerCard
        server={MOCK_SERVER}
        isInstalled={true}
        installedConfigPath="/tmp/mcp_stdio_test_server.yaml"
        updateAvailable={false}
        isInstalling={false}
        onInstall={() => undefined}
        onViewDetail={() => undefined}
        onUpdate={() => undefined}
        onUninstall={() => undefined}
      />,
    );
    expect(screen.queryByRole("button", { name: /update/i })).toBeNull();
  });
});

describe("SourceSelector", () => {
  it("renders source tabs with correct counts", () => {
    const onSelectSource = (id: string | null) => id;
    render(
      <SourceSelector
        sources={MOCK_SOURCES}
        selectedSource={null}
        onSelectSource={onSelectSource}
        onOpenSettings={() => undefined}
      />,
    );
    expect(screen.getByText(/All \(51\)/)).toBeDefined();
    expect(screen.getByText(/Refact Built-in/)).toBeDefined();
    expect(screen.getByText(/Smithery\.ai/)).toBeDefined();
  });

  it("calls onSelectSource when a source tab is clicked", () => {
    const selected: (string | null)[] = [];
    render(
      <SourceSelector
        sources={MOCK_SOURCES}
        selectedSource={null}
        onSelectSource={(id) => selected.push(id)}
        onOpenSettings={() => undefined}
      />,
    );
    const builtinBadge = screen.getByText(/Refact Built-in/);
    fireEvent.click(builtinBadge);
    expect(selected.length).toBe(1);
    expect(selected[0]).toBe("refact-bundled");
  });

  it("calls onOpenSettings when gear icon is clicked", () => {
    const opened: boolean[] = [];
    render(
      <SourceSelector
        sources={MOCK_SOURCES}
        selectedSource={null}
        onSelectSource={() => undefined}
        onOpenSettings={() => opened.push(true)}
      />,
    );
    const gearButton = screen.getByTitle("Manage marketplace sources");
    fireEvent.click(gearButton);
    expect(opened.length).toBe(1);
  });
});

describe("MCPMarketplace", () => {
  it("renders marketplace page with server cards from API", async () => {
    server.use(
      http.get("*/v1/mcp/marketplace", () => {
        return HttpResponse.json(MOCK_RESPONSE);
      }),
      http.get("*/v1/mcp/marketplace/installed", () => {
        return HttpResponse.json({ installed: [] });
      }),
    );

    render(
      <MCPMarketplace
        host="vscode"
        tabbed={false}
        backFromMarketplace={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    expect(await screen.findByText("Test Server")).toBeDefined();
    expect(screen.getByText("MCP Marketplace")).toBeDefined();
  });

  it("renders source selector tabs when sources are returned", async () => {
    server.use(
      http.get("*/v1/mcp/marketplace", () => {
        return HttpResponse.json(MOCK_RESPONSE);
      }),
      http.get("*/v1/mcp/marketplace/installed", () => {
        return HttpResponse.json({ installed: [] });
      }),
    );

    render(
      <MCPMarketplace
        host="vscode"
        tabbed={false}
        backFromMarketplace={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    await screen.findByText("Test Server");
    expect(screen.getAllByText(/Refact Built-in/).length).toBeGreaterThan(0);
    expect(screen.getByTitle("Manage marketplace sources")).toBeDefined();
  });

  it("filters servers via the engine-side q parameter", async () => {
    const secondServer: MCPServer = {
      ...MOCK_SERVER,
      id: "other-server",
      name: "Other Service",
      description: "Another service",
      tags: ["database"],
    };
    server.use(
      http.get("*/v1/mcp/marketplace", ({ request }) => {
        const url = new URL(request.url);
        const q = (url.searchParams.get("q") ?? "").toLowerCase();
        const servers = [MOCK_SERVER, secondServer].filter(
          (entry) => !q || entry.name.toLowerCase().includes(q),
        );
        return HttpResponse.json({ servers, sources: MOCK_SOURCES });
      }),
      http.get("*/v1/mcp/marketplace/installed", () => {
        return HttpResponse.json({ installed: [] });
      }),
    );

    render(
      <MCPMarketplace
        host="vscode"
        tabbed={false}
        backFromMarketplace={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    await screen.findByText("Test Server");
    expect(screen.getByText("Other Service")).toBeDefined();

    const searchInput = screen.getByPlaceholderText("Search servers…");
    fireEvent.change(searchInput, { target: { value: "Other" } });

    // Search is debounced and applied engine-side via the q parameter.
    await waitFor(() => expect(screen.queryByText("Test Server")).toBeNull());
    expect(screen.getByText("Other Service")).toBeDefined();
  });

  it("shows installed indicator for installed servers", async () => {
    server.use(
      http.get("*/v1/mcp/marketplace", () => {
        return HttpResponse.json(MOCK_RESPONSE);
      }),
      http.get("*/v1/mcp/marketplace/installed", () => {
        return HttpResponse.json({
          installed: [
            {
              id: "test-server",
              source_id: "refact-bundled",
              config_path: "/tmp/test.yaml",
            },
          ],
        });
      }),
    );

    render(
      <MCPMarketplace
        host="vscode"
        tabbed={false}
        backFromMarketplace={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    await screen.findByText("Test Server");
    expect(screen.getByText("Installed")).toBeDefined();
  });

  it("shows Smithery configure callout when Smithery source lacks API key", async () => {
    server.use(
      http.get("*/v1/mcp/marketplace", () => {
        return HttpResponse.json({
          servers: [MOCK_SERVER],
          sources: [
            ...MOCK_SOURCES.filter((s) => s.id !== "smithery"),
            { ...MOCK_SOURCES[1], enabled: true },
          ],
        });
      }),
      http.get("*/v1/mcp/marketplace/installed", () => {
        return HttpResponse.json({ installed: [] });
      }),
    );

    render(
      <MCPMarketplace
        host="vscode"
        tabbed={false}
        backFromMarketplace={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    await screen.findByText("Test Server");
    expect(
      screen.getByText(/Smithery source requires an API key/),
    ).toBeDefined();
  });

  it("shows install failures, clears installing state, and allows retry", async () => {
    let installAttempts = 0;
    const unhandledRejections: PromiseRejectionEvent[] = [];
    const onUnhandledRejection = (event: PromiseRejectionEvent) => {
      unhandledRejections.push(event);
    };
    window.addEventListener("unhandledrejection", onUnhandledRejection);

    server.use(
      http.get("*/v1/mcp/marketplace", () => {
        return HttpResponse.json({
          ...MOCK_RESPONSE,
          servers: [MOCK_SERVER_NO_REQUIRED_ENV],
        });
      }),
      http.get("*/v1/mcp/marketplace/installed", () => {
        return HttpResponse.json({ installed: [] });
      }),
      http.post("*/v1/mcp/marketplace/install", async ({ request }) => {
        installAttempts += 1;
        const body = (await request.json()) as Record<string, unknown>;
        expect(body.server_id).toBe("test-server");
        expect(body.source_id).toBe("refact-bundled");
        if (installAttempts === 1) {
          return HttpResponse.json(
            { detail: "Install exploded" },
            { status: 500 },
          );
        }
        return HttpResponse.json({
          installed: true,
          config_path: "/tmp/test.yaml",
        });
      }),
    );

    try {
      render(
        <MCPMarketplace
          host="vscode"
          tabbed={false}
          backFromMarketplace={() => undefined}
        />,
        { preloadedState: PRELOADED_STATE },
      );

      await screen.findByText("Test Server");
      fireEvent.click(screen.getByRole("button", { name: /^install$/i }));

      expect(await screen.findByText("Install exploded")).toBeDefined();
      const retryButton = screen.getByRole("button", { name: /^install$/i });
      expect(retryButton).not.toBeDisabled();
      expect(unhandledRejections).toHaveLength(0);

      fireEvent.click(retryButton);

      await waitFor(() => expect(installAttempts).toBe(2));
    } finally {
      window.removeEventListener("unhandledrejection", onUnhandledRejection);
    }
  });

  it("routes card-level install to the detail view when required env vars are empty", async () => {
    let installAttempts = 0;
    server.use(
      http.get("*/v1/mcp/marketplace", () => {
        return HttpResponse.json(MOCK_RESPONSE);
      }),
      http.get("*/v1/mcp/marketplace/installed", () => {
        return HttpResponse.json({ installed: [] });
      }),
      http.post("*/v1/mcp/marketplace/install", () => {
        installAttempts += 1;
        return HttpResponse.json({
          installed: true,
          config_path: "/tmp/test.yaml",
        });
      }),
    );

    render(
      <MCPMarketplace
        host="vscode"
        tabbed={false}
        backFromMarketplace={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    await screen.findByText("Test Server");
    fireEvent.click(screen.getByRole("button", { name: /^install$/i }));

    // Detail view with the env form opens instead of firing the install.
    expect(await screen.findByText("Configuration")).toBeDefined();
    expect(screen.getAllByText(/API_KEY/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/\(required\)/).length).toBeGreaterThan(0);
    expect(installAttempts).toBe(0);

    // Install stays disabled until the required env var is filled.
    const detailInstall = screen.getByRole("button", { name: /^install$/i });
    expect(detailInstall).toBeDisabled();
  });

  it("enables detail install once required env is filled and sends the override", async () => {
    const installBodies: Record<string, unknown>[] = [];
    server.use(
      http.get("*/v1/mcp/marketplace", () => {
        return HttpResponse.json(MOCK_RESPONSE);
      }),
      http.get("*/v1/mcp/marketplace/installed", () => {
        return HttpResponse.json({ installed: [] });
      }),
      http.post("*/v1/mcp/marketplace/install", async ({ request }) => {
        const body = (await request.json()) as Record<string, unknown>;
        installBodies.push(body);
        return HttpResponse.json({
          installed: true,
          config_path: "/tmp/test.yaml",
        });
      }),
    );

    render(
      <MCPMarketplace
        host="vscode"
        tabbed={false}
        backFromMarketplace={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    await screen.findByText("Test Server");
    fireEvent.click(screen.getByRole("button", { name: /^install$/i }));
    await screen.findByText("Configuration");

    const envInput = screen.getByRole("textbox");
    fireEvent.change(envInput, { target: { value: "sk-test-123" } });

    const detailInstall = screen.getByRole("button", { name: /^install$/i });
    expect(detailInstall).not.toBeDisabled();
    fireEvent.click(detailInstall);

    await waitFor(() => expect(installBodies).toHaveLength(1));
    expect(installBodies[0].server_id).toBe("test-server");
    const overrides = installBodies[0].config_overrides as {
      env: Record<string, string>;
    };
    expect(overrides.env.API_KEY).toBe("sk-test-123");
  });

  it("marks servers with changed recipes as updatable and fires the update call", async () => {
    const updateCalls: Record<string, unknown>[] = [];
    server.use(
      http.get("*/v1/mcp/marketplace", () => {
        return HttpResponse.json({
          ...MOCK_RESPONSE,
          servers: [{ ...MOCK_SERVER, recipe_hash: "hash-v2" }],
        });
      }),
      http.get("*/v1/mcp/marketplace/installed", () => {
        return HttpResponse.json({
          installed: [
            {
              id: "test-server",
              source_id: "refact-bundled",
              config_path: "/tmp/mcp_stdio_test_server.yaml",
              recipe_hash: "hash-v1",
            },
          ],
        });
      }),
      http.post("*/v1/mcp/marketplace/update", async ({ request }) => {
        updateCalls.push((await request.json()) as Record<string, unknown>);
        return HttpResponse.json({
          updated: true,
          config_path: "/tmp/mcp_stdio_test_server.yaml",
          recipe_hash: "hash-v2",
        });
      }),
    );

    render(
      <MCPMarketplace
        host="vscode"
        tabbed={false}
        backFromMarketplace={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    await screen.findByText("Test Server");
    const updateBtn = await screen.findByRole("button", { name: /update/i });
    fireEvent.click(updateBtn);

    await waitFor(() => expect(updateCalls).toHaveLength(1));
    expect(updateCalls[0].config_path).toBe("/tmp/mcp_stdio_test_server.yaml");
  });

  it("uses the engine-provided all_tags catalog for tag pills", async () => {
    server.use(
      http.get("*/v1/mcp/marketplace", () => {
        return HttpResponse.json({
          ...MOCK_RESPONSE,
          all_tags: ["search", "code", "tag-from-another-page"],
        });
      }),
      http.get("*/v1/mcp/marketplace/installed", () => {
        return HttpResponse.json({ installed: [] });
      }),
    );

    render(
      <MCPMarketplace
        host="vscode"
        tabbed={false}
        backFromMarketplace={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    await screen.findByText("Test Server");
    expect(screen.getByText("tag-from-another-page")).toBeDefined();
  });

  it("offers Update when the installed recipe hash differs and calls the endpoint", async () => {
    const updateCalls: Record<string, unknown>[] = [];
    server.use(
      http.get("*/v1/mcp/marketplace", () => {
        return HttpResponse.json({
          ...MOCK_RESPONSE,
          servers: [{ ...MOCK_SERVER, recipe_hash: "hash-v2" }],
        });
      }),
      http.get("*/v1/mcp/marketplace/installed", () => {
        return HttpResponse.json({
          installed: [
            {
              id: "test-server",
              source_id: "refact-bundled",
              config_path: "/tmp/mcp_stdio_test_server.yaml",
              recipe_hash: "hash-v1",
            },
          ],
        });
      }),
      http.post("*/v1/mcp/marketplace/update", async ({ request }) => {
        updateCalls.push((await request.json()) as Record<string, unknown>);
        return HttpResponse.json({
          updated: true,
          config_path: "/tmp/mcp_stdio_test_server.yaml",
          recipe_hash: "hash-v2",
        });
      }),
    );

    render(
      <MCPMarketplace
        host="vscode"
        tabbed={false}
        backFromMarketplace={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    await screen.findByText("Test Server");
    const updateBtn = await screen.findByRole("button", { name: /update/i });
    fireEvent.click(updateBtn);

    await waitFor(() => expect(updateCalls).toHaveLength(1));
    expect(updateCalls[0].config_path).toBe("/tmp/mcp_stdio_test_server.yaml");
  });

  it("uses the server-provided all_tags catalog for tag pills", async () => {
    server.use(
      http.get("*/v1/mcp/marketplace", () => {
        return HttpResponse.json({
          ...MOCK_RESPONSE,
          all_tags: ["code", "database", "search"],
        });
      }),
      http.get("*/v1/mcp/marketplace/installed", () => {
        return HttpResponse.json({ installed: [] });
      }),
    );

    render(
      <MCPMarketplace
        host="vscode"
        tabbed={false}
        backFromMarketplace={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    await screen.findByText("Test Server");
    // "database" is not on any current-page server, but comes from all_tags.
    expect(screen.getByText("database")).toBeDefined();
  });

  it("source settings dialog opens and closes", async () => {
    server.use(
      http.get("*/v1/mcp/marketplace", () => {
        return HttpResponse.json(MOCK_RESPONSE);
      }),
      http.get("*/v1/mcp/marketplace/installed", () => {
        return HttpResponse.json({ installed: [] });
      }),
    );

    render(
      <MCPMarketplace
        host="vscode"
        tabbed={false}
        backFromMarketplace={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    await screen.findByText("Test Server");
    const gearButton = screen.getByTitle("Manage marketplace sources");
    fireEvent.click(gearButton);
    expect(await screen.findByText("Marketplace Sources")).toBeDefined();

    const closeButton = screen.getByRole("button", { name: /close/i });
    fireEvent.click(closeButton);
  });
});
