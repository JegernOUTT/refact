import { describe, expect, test, vi } from "vitest";
import { http, HttpResponse } from "msw";

import { render, screen } from "../utils/test-utils";
import { server } from "../utils/mockServer";
import { setUpStore } from "../app/store";
import { getProviderName } from "../features/Providers/getProviderName";
import { ProviderCard } from "../features/Providers/ProviderCard";
import {
  isProviderDetailResponse,
  isProviderListResponse,
  providersApi,
  type ProviderListItem,
} from "../services/refact";

const aliasProvider: ProviderListItem = {
  name: "openai_work",
  base_provider: "openai",
  display_name: "Work OpenAI",
  enabled: true,
  readonly: false,
  has_credentials: true,
  status: "active",
  model_count: 2,
};

const preloadedState = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

describe("Providers provider instances", () => {
  test("getProviderName prefers display name", () => {
    expect(getProviderName(aliasProvider)).toBe("Work OpenAI");
  });

  test("ProviderCard renders alias label with instance id", () => {
    const { container } = render(
      <ProviderCard provider={aliasProvider} setCurrentProvider={vi.fn()} />,
    );

    expect(container.querySelector("svg")).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Work OpenAI" }),
    ).toBeInTheDocument();
    expect(screen.getByText("openai_work")).toBeInTheDocument();
  });

  test("provider type guards accept base provider fields", () => {
    expect(
      isProviderListResponse({
        providers: [aliasProvider],
      }),
    ).toBe(true);

    expect(
      isProviderDetailResponse({
        ...aliasProvider,
        selected_models_count: 1,
        settings: {
          base_provider: "openai",
          display_name: "Work OpenAI",
          api_key: "***",
        },
        runtime: {
          name: "openai_work",
          base_provider: "openai",
          display_name: "Work OpenAI",
          enabled: true,
          readonly: false,
          wire_format: "openai_chat_completions",
          chat_endpoint: "",
          completion_endpoint: "",
          embedding_endpoint: "",
          chat_models: [],
          completion_models: [],
          embedding_model: null,
        },
      }),
    ).toBe(true);
  });

  test("provider update payload includes identity fields", async () => {
    let requestBody: unknown;

    server.use(
      http.get("http://127.0.0.1:8001/v1/providers/openai_work", () =>
        HttpResponse.json({
          ...aliasProvider,
          selected_models_count: 1,
          settings: {
            base_provider: "openai",
            display_name: "Work OpenAI",
            api_key: "***",
          },
          runtime: null,
        }),
      ),
      http.post(
        "http://127.0.0.1:8001/v1/providers/openai_work",
        async ({ request }) => {
          requestBody = await request.json();
          return HttpResponse.json({ success: true });
        },
      ),
    );

    const store = setUpStore(preloadedState);

    try {
      const provider = await store
        .dispatch(
          providersApi.endpoints.getProvider.initiate({
            providerName: "openai_work",
          }),
        )
        .unwrap();

      await store
        .dispatch(
          providersApi.endpoints.updateProvider.initiate({
            providerName: "openai_work",
            settings: {
              base_provider: provider.base_provider,
              display_name: provider.display_name,
              api_key: "new-key",
            },
          }),
        )
        .unwrap();

      expect(requestBody).toEqual({
        base_provider: "openai",
        display_name: "Work OpenAI",
        api_key: "new-key",
      });
    } finally {
      store.dispatch(providersApi.util.resetApiState());
    }
  });
});
