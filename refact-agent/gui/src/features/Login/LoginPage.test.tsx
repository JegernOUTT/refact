import { http, HttpResponse, delay } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  activeProvider,
  capsFailsOnceThenSucceeds,
  inactiveProvider,
} from "../../__fixtures__/providers";
import { setUpStore } from "../../app/store";
import { STUB_CAPS_RESPONSE } from "../../__fixtures__/caps";
import {
  reducer as configReducer,
  updateConfig,
  type ConfigUpdate,
} from "../Config/configSlice";
import { setBackendStatus } from "../Connection";
import { act } from "react-dom/test-utils";
import { render, screen, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { LoginPage } from "./LoginPage";

function renderLoginPage(
  config: ConfigUpdate = { backendReady: true, connectionStatus: "ready" },
) {
  const store = setUpStore({
    config: {
      apiKey: "test",
      lspPort: 8001,
      themeProps: {},
      host: "vscode",
    },
  });
  store.dispatch(updateConfig(config));
  store.dispatch(setBackendStatus({ status: "online" }));
  return { ...render(<LoginPage />, { store }), store };
}

beforeEach(() => {
  vi.useRealTimers();
  server.use(
    http.get("*/v1/ping", () => HttpResponse.text("pong")),
    http.post("*/v1/buddy/diagnostics/collect", () => HttpResponse.json({})),
  );
});

describe("LoginPage provider error recovery", () => {
  it("offers an enabled Retry and keeps Continue disabled when caps fails", async () => {
    server.use(
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [inactiveProvider] }),
      ),
      http.get("*/v1/caps", () =>
        HttpResponse.json({ detail: "caps exploded" }, { status: 500 }),
      ),
    );

    renderLoginPage();

    expect(await screen.findByRole("button", { name: "Retry" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
  });

  it("re-issues providers and caps on Retry and continues after success", async () => {
    let providersHits = 0;
    const caps = capsFailsOnceThenSucceeds();
    server.use(
      http.get("*/v1/providers", () => {
        providersHits += 1;
        return HttpResponse.json({ providers: [activeProvider] });
      }),
      caps.handler,
    );

    const { user } = renderLoginPage();

    const retryButton = await screen.findByRole("button", { name: "Retry" });
    const providersHitsBeforeRetry = providersHits;

    await user.click(retryButton);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled();
    });
    expect(providersHits).toBeGreaterThan(providersHitsBeforeRetry);
    expect(caps.hits()).toBeGreaterThan(1);
    expect(
      screen.queryByRole("button", { name: "Retry" }),
    ).not.toBeInTheDocument();
  });

  it("keeps the error state with a busy Retry while the retry is in flight", async () => {
    server.use(
      http.get("*/v1/providers", async () => {
        await delay(100);
        return HttpResponse.json({ providers: [activeProvider] });
      }),
      capsFailsOnceThenSucceeds({ successDelayMs: 100 }).handler,
    );

    const { user } = renderLoginPage();

    await user.click(await screen.findByRole("button", { name: "Retry" }));

    const busyRetryButton = screen.getByRole("button", { name: "Retry" });
    expect(busyRetryButton).toBeDisabled();
    expect(busyRetryButton).toHaveAttribute("aria-busy", "true");
    expect(
      screen.getByRole("heading", { name: "Unable to Load Providers" }),
    ).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled();
    });
  });
});

describe("plugin startup prerequisites", () => {
  it.each(["connecting", "installing"] as const)(
    "keeps version disclosure and diagnostics reachable while %s",
    async (connectionStatus) => {
      const { store, user } = renderLoginPage({
        backendReady: false,
        connectionStatus,
        lspPort: 0,
        runtimeVersions: {
          requestedVersion: "wanted",
          selected: {
            state: "last_observed",
            version: "actual",
            source: "local",
          },
        },
      });
      await user.click(
        screen.getByRole("button", { name: "Component versions" }),
      );
      expect(screen.getByText("wanted")).toBeInTheDocument();
      expect(screen.getByText("actual")).toBeInTheDocument();
      expect(screen.getByText("Last observed")).toBeInTheDocument();
      await user.click(
        screen.getByRole("button", { name: "Open daemon diagnostics" }),
      );
      expect(store.getState().pages.at(-1)).toEqual({ name: "refact daemon" });
      expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    },
  );

  it.each([
    ["workspace_required", "Open a folder to use Refact"],
    ["workspace_busy", "Another Refact engine is using this folder"],
  ] as const)(
    "renders %s without offline copy, spinning, or network calls",
    async (code, title) => {
      const requests = vi.fn();
      server.use(
        http.get("*/v1/providers", () => {
          requests();
          return HttpResponse.json({ providers: [activeProvider] });
        }),
        http.get("*/v1/caps", () => {
          requests();
          return HttpResponse.json({});
        }),
        http.get("*/v1/ping", () => {
          requests();
          return HttpResponse.text("pong");
        }),
      );
      const { container, user } = renderLoginPage({
        backendReady: false,
        connectionStatus: "failed",
        lspPort: 0,
        lspUrl: null,
        browserUrl: null,
        connectionIssue: { code, holderPid: 4082485, holderHttpPort: 8005 },
      });

      expect(screen.getByRole("heading", { name: title })).toBeInTheDocument();
      expect(
        screen.queryByText(/unreachable|Connection Problem/i),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: /retry|restart|kill/i }),
      ).not.toBeInTheDocument();
      expect(container.querySelector('[aria-busy="true"]')).toBeNull();
      const continueButton = screen.getByRole("button", { name: "Continue" });
      expect(continueButton).toBeDisabled();
      await user.click(continueButton);
      if (code === "workspace_busy") {
        expect(
          screen.getByText("PID 4082485 · HTTP port 8005"),
        ).toBeInTheDocument();
        expect(
          screen.getByText(
            "Use the IDE running that engine, or close that engine yourself to let this plugin connect.",
          ),
        ).toBeInTheDocument();
      } else {
        expect(
          screen.getByText("Open a folder in your IDE."),
        ).toBeInTheDocument();
        expect(screen.queryByText(/PID 4082485/)).not.toBeInTheDocument();
      }
      await new Promise((resolve) => setTimeout(resolve, 30));
      expect(requests).not.toHaveBeenCalled();
    },
  );

  it.each([
    [
      "engine_resolution",
      "Refact could not locate or install the local engine.",
    ],
    ["daemon_start", "Refact could not start or reach the local daemon."],
    ["project_open", "The local daemon could not open this workspace."],
    ["lsp_attach", "Refact could not connect the IDE to the workspace engine."],
    ["health_check", "The workspace engine did not become ready."],
  ] as const)(
    "explains %s failures and opens the existing diagnostics route",
    async (stage, description) => {
      const { store, user } = renderLoginPage({
        backendReady: false,
        connectionStatus: "failed",
        lspPort: 0,
        connectionIssue: { code: "engine_start_failed", stage },
      });
      expect(
        screen.getByRole("heading", { name: "Refact engine could not start" }),
      ).toBeInTheDocument();
      expect(
        screen.getByText(
          `${description} Open daemon diagnostics or check your IDE logs for details.`,
        ),
      ).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
      await user.click(
        screen.getByRole("button", { name: "Open daemon diagnostics" }),
      );
      expect(store.getState().pages.at(-1)).toEqual({ name: "refact daemon" });
    },
  );

  it("omits invalid holder metadata", () => {
    renderLoginPage({
      backendReady: false,
      connectionStatus: "failed",
      lspPort: 0,
      connectionIssue: {
        code: "workspace_busy",
        holderPid: -1,
        holderHttpPort: 65536,
      },
    });
    expect(screen.queryByText(/PID|HTTP port/)).not.toBeInTheDocument();
  });

  it("still reports genuine failures without an issue as offline", () => {
    renderLoginPage({
      backendReady: false,
      connectionStatus: "failed",
      lspPort: 0,
    });
    expect(
      screen.getByRole("heading", { name: "Connection Problem" }),
    ).toBeInTheDocument();
    expect(
      screen.getAllByText("Backend server unreachable").length,
    ).toBeGreaterThan(0);
  });

  it("supports old hosts omitting the additive fields and blocks cached readiness on an issue", async () => {
    server.use(
      http.get("*/v1/caps", () => HttpResponse.json(STUB_CAPS_RESPONSE)),
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [activeProvider] }),
      ),
    );
    const { store } = renderLoginPage({});
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled(),
    );
    act(() => {
      store.dispatch(
        updateConfig({
          backendReady: false,
          connectionStatus: "failed",
          lspPort: 0,
          connectionIssue: { code: "workspace_busy" },
        }),
      );
    });
    expect(
      screen.getByRole("heading", {
        name: "Another Refact engine is using this folder",
      }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    act(() => {
      store.dispatch(
        updateConfig({
          backendReady: true,
          connectionStatus: "ready",
          lspPort: 8001,
        }),
      );
    });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled(),
    );
    expect(
      screen.queryByText("Another Refact engine is using this folder"),
    ).not.toBeInTheDocument();
  });
});

describe("connection issue config compatibility and clearing", () => {
  const issue = { code: "workspace_busy" } as const;
  const withIssue = () =>
    configReducer(undefined, updateConfig({ connectionIssue: issue }));

  it.each(["ready", "connecting", "starting", "installing"] as const)(
    "clears an omitted stale issue on %s",
    (connectionStatus) => {
      expect(
        configReducer(withIssue(), updateConfig({ connectionStatus }))
          .connectionIssue,
      ).toBeNull();
      expect(
        configReducer(
          withIssue(),
          updateConfig({ connectionStatus, connectionIssue: issue }),
        ).connectionIssue,
      ).toEqual(issue);
    },
  );

  it("preserves omission on unrelated/failed updates and allows explicit clearing", () => {
    expect(
      configReducer(undefined, updateConfig({ connectionStatus: "ready" }))
        .connectionIssue,
    ).toBeNull();
    expect(
      configReducer(withIssue(), updateConfig({ lspPort: 0 })).connectionIssue,
    ).toEqual(issue);
    expect(
      configReducer(withIssue(), updateConfig({ connectionStatus: "failed" }))
        .connectionIssue,
    ).toEqual(issue);
    expect(
      configReducer(withIssue(), updateConfig({ connectionIssue: null }))
        .connectionIssue,
    ).toBeNull();
    expect(
      configReducer(withIssue(), updateConfig({ connectionIssue: undefined }))
        .connectionIssue,
    ).toBeNull();
  });
});
