import { describe, expect, test, vi } from "vitest";
import { http, HttpResponse } from "msw";

import { render, screen, waitFor } from "../utils/test-utils";
import { server } from "../utils/mockServer";
import { setUpStore } from "../app/store";
import { getProviderName } from "../features/Providers/getProviderName";
import { ProviderCard } from "../features/Providers/ProviderCard";
import { AddProviderInstanceModal } from "../features/Providers/ProvidersView/AddProviderInstanceModal";
import { ProvidersView } from "../features/Providers/ProvidersView";
import {
  nextInstanceId,
  providerBaseOptions,
  validateProviderInstanceId,
} from "../features/Providers/ProvidersView/providerInstanceUtils";
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

const openAiProvider: ProviderListItem = {
  name: "openai",
  base_provider: "openai",
  display_name: "OpenAI",
  enabled: true,
  readonly: false,
  has_credentials: true,
  status: "active",
  model_count: 5,
};

const hiddenOpenAiResponsesProvider: ProviderListItem = {
  name: "openai_responses",
  base_provider: "openai_responses",
  display_name: "OpenAI (Responses API)",
  enabled: false,
  readonly: false,
  has_credentials: false,
  status: "not_configured",
  model_count: 0,
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
  test("nextInstanceId chooses the first unused suffix", () => {
    expect(nextInstanceId("openai", ["openai", "openai_2"])).toBe("openai_3");
  });

  test("hidden provider bases are excluded from add instance choices", () => {
    expect(
      providerBaseOptions([
        openAiProvider,
        hiddenOpenAiResponsesProvider,
        {
          ...hiddenOpenAiResponsesProvider,
          name: "xai_responses",
          base_provider: "xai_responses",
          display_name: "xAI (Responses API)",
        },
      ]),
    ).toEqual([{ id: "openai", label: "OpenAI" }]);
  });

  test("provider instance id validation matches backend shape", () => {
    for (const id of ["1openai", "OpenAI-Work", "openai_2"]) {
      expect(validateProviderInstanceId(id, [])).toBeNull();
    }

    expect(validateProviderInstanceId("_openai", [])).toBe(
      "Instance id must start with an ASCII letter or digit.",
    );
    expect(validateProviderInstanceId("openai.2", [])).toBe(
      "Instance id must not contain path characters.",
    );
    expect(validateProviderInstanceId("openai 2", [])).toBe(
      "Use ASCII letters, numbers, underscores, and hyphens only.",
    );
    expect(validateProviderInstanceId("defaults", [])).toBe(
      "This instance id is reserved.",
    );
    expect(validateProviderInstanceId("a".repeat(65), [])).toBe(
      "Instance id must be 64 characters or fewer.",
    );
    expect(validateProviderInstanceId("OPENAI", ["openai"])).toBe(
      "A provider with this id already exists.",
    );
  });

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
      http.get("*/v1/providers/openai_work", () =>
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
      http.post("*/v1/providers/openai_work", async ({ request }) => {
        requestBody = await request.json();
        return HttpResponse.json({ success: true });
      }),
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

  test("provider update invalidates available models cache", async () => {
    let availableModelsRequests = 0;

    server.use(
      http.get("*/v1/providers/openai_work/available-models", () => {
        availableModelsRequests += 1;
        return HttpResponse.json({
          models: [],
          source: "model_caps",
        });
      }),
      http.post("*/v1/providers/openai_work", () =>
        HttpResponse.json({ success: true }),
      ),
    );

    const store = setUpStore(preloadedState);

    try {
      await store
        .dispatch(
          providersApi.endpoints.getAvailableModels.initiate({
            providerName: "openai_work",
          }),
        )
        .unwrap();
      expect(availableModelsRequests).toBe(1);

      await store
        .dispatch(
          providersApi.endpoints.updateProvider.initiate({
            providerName: "openai_work",
            settings: {
              base_provider: "openai",
              display_name: "Work OpenAI",
              api_key: "new-key",
            },
          }),
        )
        .unwrap();

      await waitFor(() => expect(availableModelsRequests).toBe(2));
    } finally {
      store.dispatch(providersApi.util.resetApiState());
    }
  });

  test("provider scoped routes use instance endpoints for aliases", async () => {
    const requests: string[] = [];

    server.use(
      http.get("*/v1/providers/openrouter_work/account-info", ({ request }) => {
        requests.push(new URL(request.url).pathname);
        return HttpResponse.json({ data: {} });
      }),
      http.get("*/v1/providers/openrouter_work/health", ({ request }) => {
        requests.push(new URL(request.url).pathname);
        return HttpResponse.json({ ok: true });
      }),
      http.get("*/v1/providers/claude_code_work/usage", ({ request }) => {
        requests.push(new URL(request.url).pathname);
        return HttpResponse.json({ data: {} });
      }),
      http.get("*/v1/providers/openai_codex_work/usage", ({ request }) => {
        requests.push(new URL(request.url).pathname);
        return HttpResponse.json({ data: {} });
      }),
      http.get(
        "*/v1/providers/openrouter_work/models/openai%2Fgpt-4.1/endpoints",
        ({ request }) => {
          requests.push(new URL(request.url).pathname);
          return HttpResponse.json({
            provider_variants: [],
            available_providers: [],
          });
        },
      ),
    );

    const store = setUpStore(preloadedState);

    try {
      await store
        .dispatch(
          providersApi.endpoints.getOpenRouterAccountInfo.initiate({
            providerName: "openrouter_work",
            useInstanceRoute: true,
          }),
        )
        .unwrap();
      await store
        .dispatch(
          providersApi.endpoints.getOpenRouterHealth.initiate({
            providerName: "openrouter_work",
            useInstanceRoute: true,
          }),
        )
        .unwrap();
      await store
        .dispatch(
          providersApi.endpoints.getClaudeCodeUsage.initiate({
            providerName: "claude_code_work",
            useInstanceRoute: true,
          }),
        )
        .unwrap();
      await store
        .dispatch(
          providersApi.endpoints.getOpenAICodexUsage.initiate({
            providerName: "openai_codex_work",
            useInstanceRoute: true,
          }),
        )
        .unwrap();
      await store
        .dispatch(
          providersApi.endpoints.getOpenRouterModelEndpoints.initiate({
            providerName: "openrouter_work",
            modelId: "openai/gpt-4.1",
            useInstanceRoute: true,
          }),
        )
        .unwrap();

      expect(requests).toEqual([
        "/v1/providers/openrouter_work/account-info",
        "/v1/providers/openrouter_work/health",
        "/v1/providers/claude_code_work/usage",
        "/v1/providers/openai_codex_work/usage",
        "/v1/providers/openrouter_work/models/openai%2Fgpt-4.1/endpoints",
      ]);
    } finally {
      store.dispatch(providersApi.util.resetApiState());
    }
  });

  test("AddProviderInstanceModal submits identity fields", async () => {
    let requestBody: unknown;

    server.use(
      http.post("*/v1/providers/openai_2", async ({ request }) => {
        requestBody = await request.json();
        return HttpResponse.json({ success: true });
      }),
    );

    const onCreated = vi.fn();
    const onOpenChange = vi.fn();
    const { user, store } = render(
      <AddProviderInstanceModal
        isOpen
        configuredProviders={[openAiProvider]}
        initialBaseProvider="openai"
        onOpenChange={onOpenChange}
        onCreated={onCreated}
      />,
      { preloadedState },
    );

    expect(screen.getByDisplayValue("openai_2")).toBeInTheDocument();
    expect(
      screen.queryByText("OpenAI (Responses API)"),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Create instance" }));

    await waitFor(() => {
      expect(requestBody).toEqual({
        base_provider: "openai",
        display_name: "OpenAI 2",
        enabled: false,
      });
    });
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(onCreated).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "openai_2",
        base_provider: "openai",
        display_name: "OpenAI 2",
      }),
    );

    store.dispatch(providersApi.util.resetApiState());
  });

  test("ProvidersView hides Back button when embedded at list level", () => {
    render(
      <ProvidersView
        configuredProviders={[openAiProvider]}
        backFromProviders={vi.fn()}
        embedded
      />,
      { preloadedState },
    );
    expect(
      screen.queryByRole("button", { name: /back/i }),
    ).not.toBeInTheDocument();
  });

  test("ProvidersView shows Back button when not embedded", () => {
    render(
      <ProvidersView
        configuredProviders={[openAiProvider]}
        backFromProviders={vi.fn()}
      />,
      { preloadedState },
    );
    expect(screen.getByRole("button", { name: /back/i })).toBeInTheDocument();
  });
});
