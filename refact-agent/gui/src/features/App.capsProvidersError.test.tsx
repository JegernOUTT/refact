import { act } from "react-dom/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { setUpStore } from "../app/store";
import { InnerApp } from "./App";
import { setBackendStatus } from "./Connection";
import { setError } from "./Errors/errorsSlice";
import { popBackTo, push } from "./Pages/pagesSlice";
import { render, screen } from "../utils/test-utils";
import {
  chatSessionSubscribe,
  goodCaps,
  goodPing,
  goodUser,
  server,
  sidebarSubscribe,
} from "../utils/mockServer";

const bootstrapMock = vi.hoisted(() => ({ retry: vi.fn() }));

vi.mock("../hooks", async () => {
  const actual = await vi.importActual<typeof import("../hooks")>("../hooks");
  return {
    ...actual,
    useProviderBootstrapState: () => ({
      backendStatus: "online",
      providersQuery: {
        data: { providers: [] },
        isSuccess: true,
        isFetching: false,
      },
      capsQuery: { isSuccess: true },
      status: "ready",
      hasAnyActiveProvider: true,
      canAccessApp: true,
      canShowProviderSetup: false,
      retry: bootstrapMock.retry,
      isRetrying: false,
    }),
  };
});

vi.mock("../components/Toolbar", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    Toolbar: () => React.createElement("nav", { "data-testid": "toolbar" }),
  };
});

vi.mock("../components/MCPInteractionCenter", () => ({
  MCPInteractionCenter: () => null,
}));

vi.mock("./Workspace/WorkspaceView", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    WorkspaceView: () =>
      React.createElement("section", { "data-testid": "workspace-view" }),
  };
});

vi.mock("./Dashboard", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    Dashboard: () =>
      React.createElement("section", { "data-testid": "dashboard" }),
  };
});

vi.mock("./Login", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    LoginPage: () =>
      React.createElement("section", { "data-testid": "login-page" }),
  };
});

vi.mock("./Splash", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    SplashScreen: () =>
      React.createElement("section", { "data-testid": "splash-screen" }),
  };
});

vi.mock("./StatsDashboard", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    StatsDashboard: () => React.createElement("section"),
  };
});

vi.mock("./Settings", () => ({
  SettingsHub: () => null,
  isSettingsPage: () => false,
}));

vi.mock("./Tasks", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  const actual = await vi.importActual<typeof import("./Tasks")>("./Tasks");
  return {
    ...actual,
    TaskList: () => React.createElement("section"),
    TaskWorkspace: () => React.createElement("section"),
  };
});

vi.mock("./Knowledge", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    KnowledgeWorkspace: () => React.createElement("section"),
  };
});

vi.mock("./RefactDaemon", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    RefactDaemonPage: () => React.createElement("section"),
  };
});

vi.mock("./Buddy/BuddyHome", async () => {
  const React = await vi.importActual<typeof import("react")>("react");
  return {
    BuddyHome: () => React.createElement("section"),
  };
});

const CAPS_ERROR = "fetching caps from lsp";

function createAppStore() {
  const store = setUpStore({
    config: {
      apiKey: "test",
      lspPort: 8001,
      themeProps: {},
      host: "vscode",
    },
    pages: [{ name: "history" }],
  });
  store.dispatch(setBackendStatus({ status: "online" }));
  store.dispatch(setError({ message: CAPS_ERROR, source: "caps_providers" }));
  return store;
}

beforeEach(() => {
  bootstrapMock.retry.mockReset();
  server.use(
    goodPing,
    goodUser,
    goodCaps,
    sidebarSubscribe,
    chatSessionSubscribe,
  );
});

describe("App-level caps/providers error surface", () => {
  it("shows exactly one alert with Retry on the dashboard", async () => {
    const { user } = render(<InnerApp />, { store: createAppStore() });

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(CAPS_ERROR);
    expect(screen.getAllByRole("alert")).toHaveLength(1);

    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(bootstrapMock.retry).toHaveBeenCalledTimes(1);
  });

  it("keeps exactly one alert across a route change", async () => {
    const store = createAppStore();
    render(<InnerApp />, { store });

    await screen.findByRole("alert");

    act(() => {
      store.dispatch(push({ name: "chat" }));
    });

    expect(await screen.findByRole("alert")).toHaveTextContent(CAPS_ERROR);
    expect(screen.getAllByRole("alert")).toHaveLength(1);
  });

  it("dismisses the alert and does not bring it back", async () => {
    const { user } = render(<InnerApp />, { store: createAppStore() });

    await screen.findByRole("alert");
    await user.click(screen.getByRole("button", { name: "Dismiss" }));

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("leaves the boot-time login surface alone", async () => {
    const store = createAppStore();
    render(<InnerApp />, { store });

    await screen.findByRole("alert");

    act(() => {
      store.dispatch(popBackTo({ name: "login page" }));
    });

    expect(await screen.findByTestId("login-page")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
