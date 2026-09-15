import { http, HttpResponse, delay } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { STUB_CAPS_RESPONSE } from "../../__fixtures__/caps";
import { setUpStore } from "../../app/store";
import { updateConfig } from "../Config/configSlice";
import { setBackendStatus } from "../Connection";
import type { ProviderListItem } from "../../services/refact";
import { render, screen, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { LoginPage } from "./LoginPage";

const activeProvider: ProviderListItem = {
  name: "openai",
  base_provider: "openai",
  display_name: "OpenAI",
  enabled: true,
  readonly: false,
  has_credentials: true,
  status: "active",
  model_count: 1,
};

const inactiveProvider: ProviderListItem = {
  ...activeProvider,
  status: "configured",
};

function renderLoginPage() {
  const store = setUpStore({
    config: {
      apiKey: "test",
      lspPort: 8001,
      themeProps: {},
      host: "vscode",
    },
  });
  store.dispatch(
    updateConfig({ backendReady: true, connectionStatus: "ready" }),
  );
  store.dispatch(setBackendStatus({ status: "online" }));
  return { ...render(<LoginPage />, { store }), store };
}

beforeEach(() => {
  vi.useRealTimers();
  server.use(http.get("*/v1/ping", () => HttpResponse.text("pong")));
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
    let capsHits = 0;
    server.use(
      http.get("*/v1/providers", () => {
        providersHits += 1;
        return HttpResponse.json({ providers: [activeProvider] });
      }),
      http.get("*/v1/caps", () => {
        capsHits += 1;
        if (capsHits === 1) {
          return HttpResponse.json(
            { detail: "caps exploded" },
            { status: 500 },
          );
        }
        return HttpResponse.json(STUB_CAPS_RESPONSE);
      }),
    );

    const { user } = renderLoginPage();

    const retryButton = await screen.findByRole("button", { name: "Retry" });
    const providersHitsBeforeRetry = providersHits;

    await user.click(retryButton);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled();
    });
    expect(providersHits).toBeGreaterThan(providersHitsBeforeRetry);
    expect(capsHits).toBeGreaterThan(1);
    expect(
      screen.queryByRole("button", { name: "Retry" }),
    ).not.toBeInTheDocument();
  });

  it("keeps the error state with a busy Retry while the retry is in flight", async () => {
    let capsHits = 0;
    server.use(
      http.get("*/v1/providers", async () => {
        await delay(100);
        return HttpResponse.json({ providers: [activeProvider] });
      }),
      http.get("*/v1/caps", async () => {
        capsHits += 1;
        if (capsHits === 1) {
          return HttpResponse.json(
            { detail: "caps exploded" },
            { status: 500 },
          );
        }
        await delay(100);
        return HttpResponse.json(STUB_CAPS_RESPONSE);
      }),
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
