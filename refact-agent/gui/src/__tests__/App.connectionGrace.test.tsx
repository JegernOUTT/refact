import { act } from "react-dom/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";

import { setUpStore, type AppStore } from "../app/store";
import { InnerApp } from "../features/App";
import { setBackendStatus } from "../features/Connection";
import { render } from "../utils/test-utils";
import {
  chatSessionSubscribe,
  goodCaps,
  goodPing,
  goodUser,
  server,
  sidebarSubscribe,
} from "../utils/mockServer";

type BootstrapStatus =
  | "backend_connecting"
  | "backend_installing"
  | "backend_offline"
  | "provider_loading"
  | "provider_error"
  | "setup_required"
  | "ready";

const bootstrapMock = vi.hoisted((): { status: BootstrapStatus } => ({
  status: "ready",
}));

vi.mock("../hooks", async () => {
  const actual = await vi.importActual<typeof import("../hooks")>("../hooks");
  return {
    ...actual,
    useProviderBootstrapState: () => ({
      backendStatus: bootstrapMock.status === "ready" ? "online" : "offline",
      providersQuery: {
        data: { providers: [] },
        isSuccess: true,
        isFetching: false,
      },
      capsQuery: { isSuccess: true },
      status: bootstrapMock.status,
      hasAnyActiveProvider: true,
      canAccessApp: bootstrapMock.status === "ready",
      canShowProviderSetup: false,
    }),
  };
});

vi.mock("../components/Toolbar", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    Toolbar: () => React.createElement("nav", { "data-testid": "toolbar" }),
  };
});

vi.mock("../features/Workspace/WorkspaceView", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    WorkspaceView: () =>
      React.createElement("section", { "data-testid": "workspace-view" }),
  };
});

vi.mock("../features/Dashboard", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    Dashboard: () =>
      React.createElement("section", { "data-testid": "dashboard" }),
  };
});

vi.mock("../features/Login", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    LoginPage: () =>
      React.createElement("section", { "data-testid": "login-page" }),
  };
});

vi.mock("../features/Splash", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    SplashScreen: () =>
      React.createElement("section", { "data-testid": "splash-screen" }),
  };
});

vi.mock("../features/StatsDashboard", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    StatsDashboard: () =>
      React.createElement("section", { "data-testid": "stats-dashboard" }),
  };
});

vi.mock("../features/Settings", () => ({
  SettingsHub: () => null,
  isSettingsPage: () => false,
}));

vi.mock("../features/Tasks", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  const actual =
    await vi.importActual<typeof import("../features/Tasks")>(
      "../features/Tasks",
    );
  return {
    ...actual,
    TaskList: () => React.createElement("section"),
    TaskWorkspace: () => React.createElement("section"),
  };
});

vi.mock("../features/Knowledge", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    KnowledgeWorkspace: () => React.createElement("section"),
  };
});

vi.mock("../features/RefactDaemon", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    RefactDaemonPage: () => React.createElement("section"),
  };
});

vi.mock("../features/Buddy/BuddyHome", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    BuddyHome: () => React.createElement("section"),
  };
});

function createAppStore() {
  const store = setUpStore({
    config: {
      apiKey: "test",
      lspPort: 8001,
      themeProps: {},
      host: "vscode",
    },
    pages: [{ name: "history" }, { name: "chat" }],
  });
  store.dispatch(setBackendStatus({ status: "online" }));
  return store;
}

function lastPageName(store: AppStore) {
  const pages = store.getState().pages;
  return pages[pages.length - 1]?.name;
}

describe("App connection problem grace", () => {
  afterEach(() => {
    bootstrapMock.status = "ready";
    vi.useRealTimers();
  });

  it("does not swap to the login page for a sub-grace regression from ready", async () => {
    vi.useFakeTimers();
    server.use(
      goodPing,
      goodUser,
      goodCaps,
      sidebarSubscribe,
      chatSessionSubscribe,
    );
    const store = createAppStore();

    render(<InnerApp />, { store });

    act(() => {
      bootstrapMock.status = "backend_offline";
      store.dispatch(setBackendStatus({ status: "offline" }));
    });
    await act(async () => {
      await Promise.resolve();
    });
    act(() => {
      vi.advanceTimersByTime(2_999);
    });

    expect(lastPageName(store)).toBe("chat");

    act(() => {
      bootstrapMock.status = "ready";
      store.dispatch(setBackendStatus({ status: "online" }));
    });
    await act(async () => {
      await Promise.resolve();
    });
    act(() => {
      vi.advanceTimersByTime(5_000);
    });

    expect(lastPageName(store)).toBe("chat");
  });

  it("swaps to the login page after sustained access loss", async () => {
    vi.useFakeTimers();
    server.use(
      goodPing,
      goodUser,
      goodCaps,
      sidebarSubscribe,
      chatSessionSubscribe,
    );
    const store = createAppStore();

    render(<InnerApp />, { store });

    act(() => {
      bootstrapMock.status = "backend_offline";
      store.dispatch(setBackendStatus({ status: "offline" }));
    });
    await act(async () => {
      await Promise.resolve();
    });
    act(() => {
      vi.advanceTimersByTime(3_000);
    });

    expect(lastPageName(store)).toBe("login page");
  });
});
