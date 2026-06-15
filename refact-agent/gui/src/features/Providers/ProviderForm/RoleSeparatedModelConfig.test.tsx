import { describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";

import { fireEvent, render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { RoleSeparatedModelConfig } from "./RoleSeparatedModelConfig";
import { providersApi } from "../../../services/refact";
import type {
  ProviderDetailResponse,
  ProviderListItem,
} from "../../../services/refact";

const customProvider: ProviderListItem = {
  name: "custom_work",
  base_provider: "custom",
  display_name: "Custom Work",
  enabled: true,
  readonly: false,
  has_credentials: true,
  status: "active",
  model_count: 0,
};

const baseDetail: ProviderDetailResponse = {
  ...customProvider,
  selected_models_count: 0,
  settings: {
    base_provider: "custom",
    display_name: "Custom Work",
    api_key: "***",
  },
  runtime: null,
};

const preloadedState = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

function stubPointerCapture() {
  if (!("hasPointerCapture" in HTMLElement.prototype)) {
    Object.defineProperty(HTMLElement.prototype, "hasPointerCapture", {
      configurable: true,
      value: vi.fn(() => false),
    });
  }
}

function mockUpdateProvider(onPost: (body: unknown) => void) {
  server.use(
    http.post("*/v1/providers/custom_work", async ({ request }) => {
      onPost(await request.json());
      return HttpResponse.json({ success: true });
    }),
  );
}

describe("RoleSeparatedModelConfig", () => {
  it("does not offer unsupported openai_responses completion style", async () => {
    stubPointerCapture();
    const { user, store } = render(
      <RoleSeparatedModelConfig provider={baseDetail} />,
      {
        preloadedState,
      },
    );

    await screen.findByText("Role-separated model configuration");
    await user.click(screen.getByLabelText("Completion endpoint style"));

    expect(
      screen.getByRole("option", { name: "openai_completions" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: "openai_chat_completions" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("option", { name: "openai_responses" }),
    ).not.toBeInTheDocument();
    store.dispatch(providersApi.util.resetApiState());
  });

  it("posts exact completion endpoint, style, and model record", async () => {
    let requestBody: unknown;
    mockUpdateProvider((body) => {
      requestBody = body;
    });
    const detail = {
      ...baseDetail,
      settings: {
        ...baseDetail.settings,
        completion_endpoint_style: "openai_chat_completions",
      },
    };

    const { user, store } = render(
      <RoleSeparatedModelConfig provider={detail} />,
      {
        preloadedState,
      },
    );

    await screen.findByText("Role-separated model configuration");
    await user.clear(screen.getByLabelText("Completion endpoint"));
    await user.type(
      screen.getByLabelText("Completion endpoint"),
      "https://completion.example/v1/completions",
    );
    await user.clear(screen.getByLabelText("Completion model name"));
    await user.type(
      screen.getByLabelText("Completion model name"),
      "qwen-coder",
    );
    await user.clear(screen.getByLabelText("Completion context"));
    await user.type(screen.getByLabelText("Completion context"), "8192");
    await user.clear(screen.getByLabelText("Completion tokenizer"));
    await user.type(
      screen.getByLabelText("Completion tokenizer"),
      "hf://Qwen/Tokenizer",
    );
    await user.clear(screen.getByLabelText("Completion scratchpad"));
    await user.type(screen.getByLabelText("Completion scratchpad"), "FIM-SPM");
    fireEvent.change(screen.getByLabelText("Completion scratchpad patch"), {
      target: { value: '{"fim_prefix":"<fim>"}' },
    });
    await user.click(
      screen.getByRole("button", { name: "Save completion model" }),
    );

    await waitFor(() => {
      expect(requestBody).toEqual({
        base_provider: "custom",
        display_name: "Custom Work",
        completion_endpoint: "https://completion.example/v1/completions",
        completion_endpoint_style: "openai_chat_completions",
        completion_models: {
          "qwen-coder": {
            n_ctx: 8192,
            tokenizer: "hf://Qwen/Tokenizer",
            scratchpad: "FIM-SPM",
            scratchpad_patch: { fim_prefix: "<fim>" },
          },
        },
      });
    });
    store.dispatch(providersApi.util.resetApiState());
  });

  it("posts exact embedding endpoint, style, and model record", async () => {
    let requestBody: unknown;
    mockUpdateProvider((body) => {
      requestBody = body;
    });
    const detail = {
      ...baseDetail,
      settings: {
        ...baseDetail.settings,
        embedding_endpoint_style: "ollama_native",
      },
    };

    const { user, store } = render(
      <RoleSeparatedModelConfig provider={detail} />,
      {
        preloadedState,
      },
    );

    await screen.findByText("Role-separated model configuration");
    await user.clear(screen.getByLabelText("Embedding endpoint"));
    await user.type(
      screen.getByLabelText("Embedding endpoint"),
      "https://embedding.example/v1/embeddings",
    );
    await user.clear(screen.getByLabelText("Embedding model name"));
    await user.type(
      screen.getByLabelText("Embedding model name"),
      "nomic-embed-text",
    );
    await user.clear(screen.getByLabelText("Embedding context"));
    await user.type(screen.getByLabelText("Embedding context"), "2048");
    await user.clear(screen.getByLabelText("Embedding size"));
    await user.type(screen.getByLabelText("Embedding size"), "768");
    await user.clear(screen.getByLabelText("Embedding batch"));
    await user.type(screen.getByLabelText("Embedding batch"), "16");
    await user.clear(screen.getByLabelText("Embedding threshold"));
    await user.type(screen.getByLabelText("Embedding threshold"), "0.42");
    await user.clear(screen.getByLabelText("Embedding tokenizer"));
    await user.type(
      screen.getByLabelText("Embedding tokenizer"),
      "hf://Nomic/tokenizer",
    );
    await user.clear(screen.getByLabelText("Embedding dimensions"));
    await user.type(screen.getByLabelText("Embedding dimensions"), "512");
    await user.type(screen.getByLabelText("Embedding query prefix"), "query: ");
    await user.type(
      screen.getByLabelText("Embedding document prefix"),
      "passage: ",
    );
    await user.click(
      screen.getByRole("button", { name: "Save embedding model" }),
    );

    await waitFor(() => {
      expect(requestBody).toEqual({
        base_provider: "custom",
        display_name: "Custom Work",
        embedding_endpoint: "https://embedding.example/v1/embeddings",
        embedding_endpoint_style: "ollama_native",
        embedding_model: {
          n_ctx: 2048,
          name: "nomic-embed-text",
          tokenizer: "hf://Nomic/tokenizer",
          embedding_size: 768,
          embedding_batch: 16,
          rejection_threshold: 0.42,
          dimensions: 512,
          query_prefix: "query: ",
          document_prefix: "passage: ",
        },
      });
    });
    store.dispatch(providersApi.util.resetApiState());
  });

  it("populates the embedding model name from a legacy string-shaped config", async () => {
    const detail = {
      ...baseDetail,
      settings: {
        ...baseDetail.settings,
        embedding_endpoint: "https://embedding.example/v1/embeddings",
        embedding_model: "legacy-embed-name",
      },
    };

    const { store } = render(<RoleSeparatedModelConfig provider={detail} />, {
      preloadedState,
    });

    await screen.findByText("Role-separated model configuration");
    expect(
      await screen.findByDisplayValue("legacy-embed-name"),
    ).toBeInTheDocument();
    store.dispatch(providersApi.util.resetApiState());
  });

  it("populates the embedding model name from an object-shaped config", async () => {
    const detail = {
      ...baseDetail,
      settings: {
        ...baseDetail.settings,
        embedding_model: { name: "object-embed-name", embedding_size: 768 },
      },
    };

    const { store } = render(<RoleSeparatedModelConfig provider={detail} />, {
      preloadedState,
    });

    await screen.findByText("Role-separated model configuration");
    expect(
      await screen.findByDisplayValue("object-embed-name"),
    ).toBeInTheDocument();
    store.dispatch(providersApi.util.resetApiState());
  });

  it("editing completion and embedding records does not post chat custom models", async () => {
    const requests: unknown[] = [];
    mockUpdateProvider((body) => requests.push(body));
    const detail = {
      ...baseDetail,
      settings: {
        ...baseDetail.settings,
        custom_models: {
          chatty: { n_ctx: 4096 },
        },
        completion_models: {
          coder: { n_ctx: 4096, tokenizer: "hf://old", scratchpad: "FIM-PSM" },
        },
        embedding_model: {
          name: "embed-old",
          n_ctx: 512,
          tokenizer: "hf://embed",
          embedding_size: 384,
          embedding_batch: 4,
          rejection_threshold: 0.2,
        },
      },
    };

    const { user, store } = render(
      <RoleSeparatedModelConfig provider={detail} />,
      {
        preloadedState,
      },
    );

    await screen.findByDisplayValue("coder");
    await user.click(
      screen.getByRole("button", { name: "Save completion model" }),
    );
    await user.click(
      screen.getByRole("button", { name: "Save embedding model" }),
    );

    await waitFor(() => {
      expect(requests).toHaveLength(2);
    });
    expect(requests[0]).not.toHaveProperty("custom_models");
    expect(requests[1]).not.toHaveProperty("custom_models");
    store.dispatch(providersApi.util.resetApiState());
  });

  it("rejects malformed numeric fields", async () => {
    const requests: unknown[] = [];
    mockUpdateProvider((body) => requests.push(body));

    const { user, store } = render(
      <RoleSeparatedModelConfig provider={baseDetail} />,
      {
        preloadedState,
      },
    );

    await screen.findByText("Role-separated model configuration");
    await user.type(screen.getByLabelText("Completion model name"), "coder");
    await user.clear(screen.getByLabelText("Completion context"));
    await user.type(screen.getByLabelText("Completion context"), "0");
    await user.click(
      screen.getByRole("button", { name: "Save completion model" }),
    );

    expect(
      await screen.findByText("Completion context must be a positive integer."),
    ).toBeInTheDocument();
    expect(requests).toHaveLength(0);
    store.dispatch(providersApi.util.resetApiState());
  });
});
