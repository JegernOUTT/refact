import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { STUB_CAPS_RESPONSE } from "../../__fixtures__/caps";
import { setUpStore } from "../../app/store";
import { updateConfig } from "../Config/configSlice";
import { setBackendStatus } from "../Connection";
import { useProviderBootstrapState } from "../../hooks";
import type { ProviderListItem } from "../../services/refact";
import { render, screen, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { CapsProvidersErrorBanner } from "./CapsProvidersErrorBanner";
import { setError } from "./errorsSlice";

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

const Harness = () => {
  const { retry, isRetrying } = useProviderBootstrapState();
  return <CapsProvidersErrorBanner onRetry={retry} isRetrying={isRetrying} />;
};

function renderBanner() {
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
  return { ...render(<Harness />, { store }), store };
}

beforeEach(() => {
  vi.useRealTimers();
  server.use(http.get("*/v1/ping", () => HttpResponse.text("pong")));
});

describe("CapsProvidersErrorBanner", () => {
  it("surfaces a post-boot caps failure exactly once with a Retry", async () => {
    server.use(
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [activeProvider] }),
      ),
      http.get("*/v1/caps", () =>
        HttpResponse.json({ detail: "caps exploded" }, { status: 500 }),
      ),
    );

    renderBanner();

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("caps exploded");
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
  });

  it("re-issues caps and providers on Retry and clears on success", async () => {
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

    const { user, store } = renderBanner();

    const retryButton = await screen.findByRole("button", { name: "Retry" });
    const providersHitsBeforeRetry = providersHits;

    await user.click(retryButton);

    await waitFor(() => {
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });
    expect(providersHits).toBeGreaterThan(providersHitsBeforeRetry);
    expect(capsHits).toBeGreaterThan(1);
    expect(store.getState().error.capsProviders.message).toBeNull();
  });

  it("stays dismissed once the user dismisses it", async () => {
    server.use(
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [activeProvider] }),
      ),
      http.get("*/v1/caps", () =>
        HttpResponse.json({ detail: "caps exploded" }, { status: 500 }),
      ),
    );

    const { user } = renderBanner();

    await user.click(await screen.findByRole("button", { name: "Dismiss" }));

    await waitFor(() => {
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });
  });

  it("dismisses the banner without dropping a stored route error", async () => {
    server.use(
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [activeProvider] }),
      ),
      http.get("*/v1/caps", () =>
        HttpResponse.json({ detail: "caps exploded" }, { status: 500 }),
      ),
    );

    const { user, store } = renderBanner();

    await screen.findByRole("alert");
    store.dispatch(setError("deleting integrations."));

    await user.click(screen.getByRole("button", { name: "Dismiss" }));

    await waitFor(() => {
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });
    expect(store.getState().error.message).toBe("deleting integrations.");
  });

  it("keeps a route error out of the banner while a caps failure is stored", async () => {
    server.use(
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [activeProvider] }),
      ),
      http.get("*/v1/caps", () =>
        HttpResponse.json({ detail: "caps exploded" }, { status: 500 }),
      ),
    );

    const { store } = renderBanner();

    await screen.findByRole("alert");
    store.dispatch(setError("deleting integrations."));

    await waitFor(() => {
      expect(store.getState().error.message).toBe("deleting integrations.");
    });
    expect(store.getState().error.capsProviders.message).toBe("caps exploded");
    expect(screen.getByRole("alert")).toHaveTextContent("caps exploded");
    expect(screen.getAllByRole("alert")).toHaveLength(1);
  });

  it("ignores errors that belong to a route-local surface", async () => {
    server.use(
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [activeProvider] }),
      ),
      http.get("*/v1/caps", () => HttpResponse.json(STUB_CAPS_RESPONSE)),
    );

    const { store } = renderBanner();
    store.dispatch(setError("deleting integrations."));

    await waitFor(() => {
      expect(store.getState().error.message).toBe("deleting integrations.");
    });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
