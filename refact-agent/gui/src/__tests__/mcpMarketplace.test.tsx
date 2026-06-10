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

  it("renders verified badge when server is verified", () => {
    const verifiedServer = { ...MOCK_SERVER, verified: true };
    render(
      <ServerCard
        server={verifiedServer}
        isInstalled={false}
        isInstalling={false}
        onInstall={() => undefined}
        onViewDetail={() => undefined}
      />,
    );
    expect(screen.getByText("Verified")).toBeDefined();
  });

  it("renders use count when provided", () => {
    const countedServer = { ...MOCK_SERVER, use_count: 42 };
    render(
      <ServerCard
        server={countedServer}
        isInstalled={false}
        isInstalling={false}
        onInstall={() => undefined}
        onViewDetail={() => undefined}
      />,
    );
    expect(screen.getByText("42 installs")).toBeDefined();
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

  it("filters servers by search query", async () => {
    const secondServer: MCPServer = {
      ...MOCK_SERVER,
      id: "other-server",
      name: "Other Service",
      description: "Another service",
      tags: ["database"],
    };
    server.use(
      http.get("*/v1/mcp/marketplace", () => {
        return HttpResponse.json({
          servers: [MOCK_SERVER, secondServer],
          sources: MOCK_SOURCES,
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
    expect(screen.getByText("Other Service")).toBeDefined();

    const searchInput = screen.getByPlaceholderText("Search servers…");
    fireEvent.change(searchInput, { target: { value: "Other" } });

    expect(screen.queryByText("Test Server")).toBeNull();
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
              name: "Test Server",
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
        return HttpResponse.json(MOCK_RESPONSE);
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
