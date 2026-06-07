import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";

import { render, screen } from "../../../../../utils/test-utils";
import { server } from "../../../../../utils/mockServer";
import { ModelCardPopup } from "./ModelCardPopup";
import { modelsApi } from "../../../../../services/refact";
import type {
  CodeCompletionModel,
  EmbeddingModel,
  Model,
} from "../../../../../services/refact";

const preloadedState = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

const completionModel: CodeCompletionModel = {
  name: "coder",
  n_ctx: 4096,
  tokenizer: "hf://Coder",
  completion_endpoint_style: "openai_chat_completions",
  scratchpad: "FIM-SPM",
  scratchpad_patch: { fim_prefix: "<fim>" },
  model_family: null,
  type: "completion",
  enabled: true,
};

const embeddingModel: EmbeddingModel = {
  name: "embedder",
  id: "custom/embedder",
  n_ctx: 512,
  tokenizer: "hf://Embedder",
  embedding_endpoint_style: "openai",
  embedding_size: 768,
  rejection_threshold: 0.3,
  embedding_batch: 8,
  type: "embedding",
  enabled: true,
};

function mockModel(model: Model) {
  server.use(
    http.get("*/v1/model", () => HttpResponse.json(model)),
    http.get("*/v1/model-defaults", () => HttpResponse.json(model)),
    http.get("*/v1/completion-model-families", () =>
      HttpResponse.json({ model_families: ["qwen2.5-coder-base"] }),
    ),
  );
}

describe("ModelCardPopup", () => {
  it("explains completion endpoint style separately from scratchpad style", async () => {
    mockModel(completionModel);
    const { store } = render(
      <ModelCardPopup
        isOpen
        isSaving={false}
        setIsOpen={() => undefined}
        onSave={async () => true}
        onUpdate={async () => true}
        modelName="coder"
        modelType="completion"
        providerName="custom_work"
        currentModelNames={["coder"]}
      />,
      { preloadedState },
    );

    expect(
      await screen.findByText(/Endpoint style chooses the completion HTTP API/),
    ).toBeInTheDocument();
    store.dispatch(modelsApi.util.resetApiState());
  });

  it("explains OpenAI-compatible and Ollama-native embedding styles", async () => {
    mockModel(embeddingModel);
    const { store } = render(
      <ModelCardPopup
        isOpen
        isSaving={false}
        setIsOpen={() => undefined}
        onSave={async () => true}
        onUpdate={async () => true}
        modelName="embedder"
        modelType="embedding"
        providerName="custom_work"
        currentModelNames={["embedder"]}
      />,
      { preloadedState },
    );

    expect(
      await screen.findByText(/OpenAI-compatible embeddings use OpenAI-style/),
    ).toBeInTheDocument();
    store.dispatch(modelsApi.util.resetApiState());
  });
});
