import { http, HttpResponse, delay } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { STUB_CAPS_RESPONSE } from "../__fixtures__/caps";
import { setUpStore } from "../app/store";
import { setBackendStatus } from "../features/Connection";
import { LoginPage } from "../features/Login";
import {
  capsApi,
  providersApi,
  type ProviderListItem,
} from "../services/refact";
import { render, screen, waitFor } from "../utils/test-utils";
import { server } from "../utils/mockServer";

const inactiveProvider: ProviderListItem = {
  name: "openai",
  base_provider: "openai",
  display_name: "OpenAI",
  enabled: true,
  readonly: false,
  has_credentials: true,
  status: "configured",
  model_count: 1,
};

function createStore() {
  return setUpStore({
    config: {
      apiKey: "test",
      lspPort: 8001,
      themeProps: {},
      host: "vscode",
    },
  });
}

function renderLoginPage() {
  const store = createStore();
  const view = render(<LoginPage />, { store });
  return {
    ...view,
    store,
  };
}

beforeEach(() => {
  vi.useRealTimers();
  server.use(
    http.get("*/v1/ping", async () => {
      await delay(100);
      return HttpResponse.text("pong");
    }),
    http.get("*/v1/caps", () => HttpResponse.json(STUB_CAPS_RESPONSE)),
  );
});

function mockCapsReady(store: ReturnType<typeof createStore>) {
  void store.dispatch(
    capsApi.util.upsertQueryData("getCaps", undefined, STUB_CAPS_RESPONSE),
  );
}

describe("provider bootstrap gate", () => {
  it("shows connecting state instead of provider setup while backend status is unknown", () => {
    const { store } = renderLoginPage();

    store.dispatch(setBackendStatus({ status: "unknown" }));

    expect(
      screen.getByRole("heading", { name: "Connecting to Refact" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Set Up Providers" }),
    ).not.toBeInTheDocument();
  });

  it("shows connection problem instead of provider setup while backend is offline", async () => {
    const { store } = renderLoginPage();

    store.dispatch(setBackendStatus({ status: "offline" }));

    expect(
      await screen.findByRole("heading", { name: "Connection Problem" }),
    ).toBeInTheDocument();
    expect(
      screen.getAllByText("Backend server unreachable").length,
    ).toBeGreaterThan(0);
    expect(
      screen.queryByRole("heading", { name: "Set Up Providers" }),
    ).not.toBeInTheDocument();
  });

  it("shows loading state instead of provider setup while providers are loading", async () => {
    server.use(
      http.get("*/v1/providers", async () => {
        await delay(200);
        return HttpResponse.json({ providers: [inactiveProvider] });
      }),
    );

    const { store } = renderLoginPage();

    store.dispatch(setBackendStatus({ status: "online" }));

    expect(
      await screen.findByRole("heading", { name: "Loading Providers" }),
    ).toBeInTheDocument();
    mockCapsReady(store);

    expect(
      screen.queryByRole("heading", { name: "Set Up Providers" }),
    ).not.toBeInTheDocument();
  });

  it("shows provider setup only after providers resolve with no active provider", async () => {
    server.use(
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [inactiveProvider] }),
      ),
    );

    const { store } = renderLoginPage();

    store.dispatch(setBackendStatus({ status: "online" }));

    expect(
      await screen.findByRole("heading", { name: "Set Up Providers" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /OpenAI/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
  });

  it("clears stale provider setup state when backend leaves online", async () => {
    server.use(
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [inactiveProvider] }),
      ),
    );

    const { store } = renderLoginPage();

    store.dispatch(setBackendStatus({ status: "online" }));
    void store.dispatch(
      providersApi.util.upsertQueryData("getConfiguredProviders", undefined, {
        providers: [inactiveProvider],
        error_log: [],
      }),
    );

    expect(
      await screen.findByRole("heading", { name: "Set Up Providers" }),
    ).toBeInTheDocument();

    store.dispatch(setBackendStatus({ status: "offline" }));

    await waitFor(() => {
      expect(
        screen.getByRole("heading", { name: "Connection Problem" }),
      ).toBeInTheDocument();
    });
    expect(
      screen.queryByRole("heading", { name: "Set Up Providers" }),
    ).not.toBeInTheDocument();
    expect(
      providersApi.endpoints.getConfiguredProviders.select(undefined)(
        store.getState(),
      ).data,
    ).toBeUndefined();
  });
});
