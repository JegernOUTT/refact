import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";

import { type AppStore, setUpStore } from "../app/store";
import { EditTool } from "../components/ChatContent/ToolCard/EditTool";
import { InnerApp } from "../features/App";
import { createChatWithId } from "../features/Chat/Thread";
import {
  selectCapabilities,
  selectSurface,
  type Config,
} from "../features/Config/configSlice";
import { setBackendStatus } from "../features/Connection";
import { makeSurfaceKey } from "../features/Workspace";
import type { DiffChunk, ToolCall } from "../services/refact/types";
import {
  setProjectStorageNamespace,
  setProjectStorageNamespaceFromProjectInfo,
} from "../utils/chatUiPersistence";
import { render, screen } from "../utils/test-utils";
import {
  chatLinks,
  chatSessionAbort,
  chatSessionCommand,
  chatSessionSubscribe,
  emptyTasks,
  goodCaps,
  goodPing,
  goodPrompts,
  goodTools,
  goodUser,
  noCommandPreview,
  noCompletions,
  server,
  sidebarSubscribe,
} from "../utils/mockServer";

vi.mock("../features/Chat/Chat", async () => {
  const React = await vi.importActual<typeof import("react")>("react");

  return {
    Chat: ({ chatId }: { chatId?: string }) =>
      React.createElement(
        "section",
        { "data-testid": "chat-surface", "data-chat-id": chatId ?? "" },
        `Chat surface ${chatId ?? ""}`,
      ),
  };
});

class QuietEventSource {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 2;
  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readonly CLOSED = 2;
  onerror: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onopen: ((event: Event) => void) | null = null;
  readyState = QuietEventSource.CONNECTING;
  url: string;

  constructor(url: string | URL) {
    this.url = String(url);
  }

  addEventListener() {
    return undefined;
  }

  removeEventListener() {
    return undefined;
  }

  close() {
    this.readyState = QuietEventSource.CLOSED;
  }

  dispatchEvent() {
    return true;
  }
}

const appHandlers = [
  goodPing,
  goodUser,
  goodCaps,
  goodTools,
  goodPrompts,
  chatLinks,
  chatSessionSubscribe,
  chatSessionCommand,
  chatSessionAbort,
  emptyTasks,
  noCommandPreview,
  noCompletions,
  sidebarSubscribe,
  http.get("*/v1/chat-modes", () =>
    HttpResponse.json({ modes: [], errors: [] }),
  ),
  http.get("*/v1/setup/status", () =>
    HttpResponse.json({
      configured: true,
      reasons: [],
      detail: {
        project_root: "/tmp/refact-test",
        has_agents_md: true,
        has_knowledge: true,
        has_trajectories: true,
      },
    }),
  ),
  http.get("*/v1/voice/status", () => HttpResponse.json({ available: false })),
  http.get("*/v1/chats/:chatId/skills-status", () =>
    HttpResponse.json({
      skills_available: 0,
      skills_included: [],
      skills_enabled: false,
      active_skill: null,
    }),
  ),
  http.get("*/v1/buddy/opportunities", () =>
    HttpResponse.json({ opportunities: [] }),
  ),
  http.get("*/v1/worktrees", () =>
    HttpResponse.json({
      project_hash: "test",
      source_workspace_root: "/tmp/refact-test",
      worktrees: [],
    }),
  ),
  http.get("*/daemon/v1/status", () =>
    HttpResponse.json({
      pid: 10,
      version: "9.1.0",
      port: 8488,
      started_at_ms: 1,
      uptime_secs: 3_700,
      workers: 2,
      cron_pending: {},
    }),
  ),
  http.get("*/daemon/v1/workers", () => HttpResponse.json([])),
  http.get(
    "*/daemon/v1/events",
    () =>
      new HttpResponse("", {
        headers: { "Content-Type": "text/event-stream" },
      }),
  ),
];

function makeConfig(overrides: Partial<Config>): Config {
  return {
    host: "vscode",
    lspPort: 8001,
    apiKey: "test",
    themeProps: {},
    ...overrides,
  };
}

function renderApp(
  configOverrides: Partial<Config>,
  setup?: (store: AppStore) => void,
) {
  server.use(...appHandlers);
  setProjectStorageNamespaceFromProjectInfo({
    workspaceRoots: ["/tmp/refact-test"],
    projectName: "refact-test",
  });
  const store = setUpStore({
    config: makeConfig(configOverrides),
    current_project: {
      name: "refact-test",
      workspaceRoots: ["/tmp/refact-test"],
    },
    pages: [{ name: "history" }, { name: "chat" }],
  });
  store.dispatch(setBackendStatus({ status: "online" }));
  setup?.(store);

  return render(<InnerApp />, { store });
}

function expectNoDashboardChrome() {
  expect(screen.queryByTestId("daemon-dashboard-shell")).toBeNull();
  expect(
    screen.queryByRole("navigation", { name: "Dashboard navigation" }),
  ).toBeNull();
  expect(screen.queryByText("Mission control")).toBeNull();
  expect(screen.queryByText("Daemon dashboard")).toBeNull();
}

function expectNoPanelChrome() {
  expect(
    screen.getByRole("button", { name: "Workspace panels" }),
  ).toHaveAttribute("aria-pressed", "false");
  expect(screen.queryByRole("tab", { name: "Files" })).toBeNull();
  expect(screen.queryByRole("tab", { name: "Git" })).toBeNull();
  expect(screen.queryByRole("tab", { name: "Terminal" })).toBeNull();
  expect(
    screen.queryByRole("button", { name: "Toggle workspace dock" }),
  ).toBeNull();
  expect(screen.queryByLabelText("Terminal drawer")).toBeNull();
}

afterEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  setProjectStorageNamespace(undefined);
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

describe("IDE characterization: zero new chrome", () => {
  const ideHosts = [["vscode"], ["jetbrains"]] as const;

  it.each(ideHosts)(
    "renders the %s host with only the opt-in panel button",
    async (host) => {
      const { store } = renderApp({ host }, (appStore) => {
        appStore.dispatch(
          createChatWithId({
            id: "chat-a",
            title: "Chat Alpha",
            mode: "agent",
          }),
        );
      });

      await screen.findByRole("tab", { name: /Chat Alpha/ });

      expectNoDashboardChrome();
      expectNoPanelChrome();
      expect(
        screen.getByRole("tablist", { name: "Open workspace tabs" }),
      ).toBeInTheDocument();
      expect(store.getState().workspace.tabs).toEqual([
        makeSurfaceKey("chat", "chat-a"),
      ]);
    },
  );

  it.each(ideHosts)(
    "mounts workspace panels after opting in on %s",
    async (host) => {
      const { user } = renderApp({ host }, (appStore) => {
        appStore.dispatch(
          createChatWithId({
            id: "chat-a",
            title: "Chat Alpha",
            mode: "agent",
          }),
        );
      });

      await screen.findByRole("tab", { name: /Chat Alpha/ });
      expect(screen.queryByLabelText("Workspace dock")).toBeNull();
      expect(screen.queryByLabelText("Terminal drawer")).toBeNull();

      await user.click(
        screen.getByRole("button", { name: "Workspace panels" }),
      );

      expect(
        await screen.findByLabelText("Workspace dock"),
      ).toBeInTheDocument();
      expect(screen.getByLabelText("Terminal drawer")).toBeInTheDocument();
      expect(screen.getByRole("radio", { name: "Files" })).toBeInTheDocument();
      expect(screen.getByRole("radio", { name: "Git" })).toBeInTheDocument();
      expect(screen.getByText("Terminal")).toBeInTheDocument();
    },
  );

  it.each(ideHosts)(
    "defaults the %s host to the workspace surface with IDE capabilities",
    async (host) => {
      const { store } = renderApp({ host }, (appStore) => {
        appStore.dispatch(
          createChatWithId({
            id: "chat-a",
            title: "Chat Alpha",
            mode: "agent",
          }),
        );
      });

      await screen.findByRole("tab", { name: /Chat Alpha/ });

      expect(selectSurface(store.getState())).toBe("workspace");
      expect(selectCapabilities(store.getState())).toEqual({
        filesPanel: false,
        gitPanel: false,
        terminalPanel: false,
        openFileInApp: false,
        openFileInIde: true,
        ideDiffPasteBack: true,
        folderPicker: false,
      });
    },
  );

  it("renders the dashboard on an IDE host only when surface is explicitly passed", () => {
    vi.stubGlobal("EventSource", QuietEventSource);
    server.use(...appHandlers);
    const store = setUpStore({
      config: makeConfig({ host: "vscode", surface: "dashboard" }),
    });

    render(<InnerApp />, { store });

    expect(selectSurface(store.getState())).toBe("dashboard");
    expect(screen.getByTestId("daemon-dashboard-shell")).toBeInTheDocument();
    expect(
      screen.queryByRole("tablist", { name: "Open workspace tabs" }),
    ).toBeNull();
  });

  it("never derives the dashboard surface from the host alone", () => {
    const hosts = ["web", "ide", "vscode", "jetbrains"] as const;

    for (const host of hosts) {
      const store = setUpStore({ config: makeConfig({ host }) });
      expect(selectSurface(store.getState())).toBe("workspace");
    }
  });
});

describe("IDE characterization: diff paste-back affordance", () => {
  it("keeps the Apply diff action for IDE hosts with ideDiffPasteBack", () => {
    const toolCall: ToolCall = {
      id: "edit-ide",
      index: 0,
      function: {
        name: "patch",
        arguments: JSON.stringify({
          path: "src/demo.ts",
          old_str: "old",
          replacement: "new",
        }),
      },
    };
    const diff: DiffChunk = {
      file_name: "src/demo.ts",
      file_action: "edit",
      line1: 1,
      line2: 1,
      lines_remove: "old\n",
      lines_add: "new\n",
    };

    render(
      <EditTool toolCall={toolCall} diffs={[diff]} isActiveTool={false} />,
      { preloadedState: { config: makeConfig({ host: "vscode" }) } },
    );

    expect(
      screen.getByRole("button", { name: "Apply diff" }),
    ).toBeInTheDocument();
  });
});
