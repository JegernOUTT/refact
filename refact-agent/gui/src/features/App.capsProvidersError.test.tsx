import { act } from "react-dom/test-utils";
import { http, HttpResponse } from "msw";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { STUB_CAPS_RESPONSE } from "../__fixtures__/caps";
import { activeProvider } from "../__fixtures__/providers";
import { setUpStore, type AppStore } from "../app/store";
import { InnerApp } from "./App";
import { updateConfig } from "./Config/configSlice";
import { setBackendStatus } from "./Connection";
import { capsApi, providersApi } from "../services/refact";
import { render, screen, waitFor } from "../utils/test-utils";
import {
  chatSessionSubscribe,
  goodPing,
  goodUser,
  server,
  sidebarSubscribe,
} from "../utils/mockServer";

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

const CAPS_ERROR = "caps exploded";
const LOGIN_HEADING = "Unable to Load Providers";
const ACCESS_LOSS_GRACE_MS = 3_000;

type CapsOutcome = "ok" | "error";

function useCapsHandler(outcome: { current: CapsOutcome }) {
  return http.get("*/v1/caps", () =>
    outcome.current === "error"
      ? HttpResponse.json({ detail: CAPS_ERROR }, { status: 500 })
      : HttpResponse.json(STUB_CAPS_RESPONSE),
  );
}

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
  store.dispatch(
    updateConfig({ backendReady: true, connectionStatus: "ready" }),
  );
  store.dispatch(setBackendStatus({ status: "online" }));
  return store;
}

function lastPageName(store: AppStore) {
  const pages = store.getState().pages;
  return pages[pages.length - 1]?.name;
}

async function waitForBootstrap(store: AppStore) {
  await waitFor(() => {
    const state = store.getState();
    expect(capsApi.endpoints.getCaps.select(undefined)(state).isSuccess).toBe(
      true,
    );
    expect(
      providersApi.endpoints.getConfiguredProviders.select(undefined)(state)
        .isSuccess,
    ).toBe(true);
  });
}

beforeEach(() => {
  vi.useRealTimers();
  server.use(goodPing, goodUser, sidebarSubscribe, chatSessionSubscribe);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("App-level caps/providers error surface", () => {
  it("keeps the current page and shows the banner when caps fails after bootstrap", async () => {
    const caps: { current: CapsOutcome } = { current: "ok" };
    server.use(
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [activeProvider] }),
      ),
      useCapsHandler(caps),
    );
    const store = createAppStore();

    render(<InnerApp />, { store });
    await waitForBootstrap(store);

    caps.current = "error";
    vi.useFakeTimers({ shouldAdvanceTime: true });
    await act(async () => {
      await store.dispatch(
        capsApi.endpoints.getCaps.initiate(undefined, { forceRefetch: true }),
      );
    });

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(CAPS_ERROR);
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();

    act(() => {
      vi.advanceTimersByTime(ACCESS_LOSS_GRACE_MS * 2);
    });

    expect(lastPageName(store)).toBe("history");
    expect(
      screen.queryByRole("heading", { name: LOGIN_HEADING }),
    ).not.toBeInTheDocument();
    expect(screen.getByTestId("dashboard")).toBeInTheDocument();
    expect(screen.getAllByRole("alert")).toHaveLength(1);
  });

  it("clears the banner after a successful retry and stays on the page", async () => {
    const caps: { current: CapsOutcome } = { current: "ok" };
    server.use(
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [activeProvider] }),
      ),
      useCapsHandler(caps),
    );
    const store = createAppStore();

    const { user } = render(<InnerApp />, { store });
    await waitForBootstrap(store);

    caps.current = "error";
    await act(async () => {
      await store.dispatch(
        capsApi.endpoints.getCaps.initiate(undefined, { forceRefetch: true }),
      );
    });
    await screen.findByRole("alert");

    caps.current = "ok";
    await user.click(screen.getByRole("button", { name: "Retry" }));

    await waitFor(() => {
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });
    expect(lastPageName(store)).toBe("history");
    expect(screen.getByTestId("dashboard")).toBeInTheDocument();
  });

  it("falls back to the login page when caps fails during boot", async () => {
    const caps: { current: CapsOutcome } = { current: "error" };
    server.use(
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [activeProvider] }),
      ),
      useCapsHandler(caps),
    );
    const store = createAppStore();

    render(<InnerApp />, { store });

    expect(
      await screen.findByRole("heading", { name: LOGIN_HEADING }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(lastPageName(store)).toBe("login page");
  });

  it("still pops back to the login page when the session loses authorization", async () => {
    const caps: { current: CapsOutcome } = { current: "ok" };
    let providersUnauthorized = false;
    server.use(
      http.get("*/v1/providers", () =>
        providersUnauthorized
          ? HttpResponse.json({ detail: "unauthorized" }, { status: 401 })
          : HttpResponse.json({ providers: [activeProvider] }),
      ),
      useCapsHandler(caps),
    );
    const store = createAppStore();

    render(<InnerApp />, { store });
    await waitForBootstrap(store);

    providersUnauthorized = true;
    vi.useFakeTimers({ shouldAdvanceTime: true });
    await act(async () => {
      await store.dispatch(
        providersApi.endpoints.getConfiguredProviders.initiate(undefined, {
          forceRefetch: true,
        }),
      );
    });

    act(() => {
      vi.advanceTimersByTime(ACCESS_LOSS_GRACE_MS);
    });

    expect(lastPageName(store)).toBe("login page");
  });
});
