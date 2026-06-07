import { useCallback, useMemo, useState } from "react";
import {
  Button,
  Card,
  Flex,
  Select,
  Text,
  TextArea,
  TextField,
} from "@radix-ui/themes";

import type {
  CompletionProviderModelConfig,
  EmbeddingProviderModelConfig,
  ProviderDetailResponse,
  ProviderFormRoleSettings,
} from "../../../services/refact";
import {
  providersApi,
  providerIdentitySettings,
} from "../../../services/refact";

const COMPLETION_ENDPOINT_STYLES = [
  "openai_completions",
  "openai_chat_completions",
];
const EMBEDDING_ENDPOINT_STYLES = ["openai", "ollama_native", "voyage"];
const DEFAULT_COMPLETION_CONTEXT = "4096";
const DEFAULT_EMBEDDING_CONTEXT = "512";
const DEFAULT_EMBEDDING_SIZE = "1536";
const DEFAULT_EMBEDDING_BATCH = "8";
const DEFAULT_REJECTION_THRESHOLD = "0.3";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function stringValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function numberValue(value: unknown, fallback: string): string {
  return typeof value === "number" && Number.isFinite(value)
    ? String(value)
    : fallback;
}

function optionalNumberValue(value: unknown): string {
  return typeof value === "number" && Number.isFinite(value)
    ? String(value)
    : "";
}

function parseInteger(value: string): number | null {
  const parsed = Number(value.trim());
  if (!Number.isInteger(parsed) || parsed <= 0) return null;
  return parsed;
}

function parsePositiveNumber(value: string): number | null {
  const parsed = Number(value.trim());
  if (!Number.isFinite(parsed) || parsed <= 0) return null;
  return parsed;
}

function parseOptionalInteger(value: string): number | undefined | null {
  if (!value.trim()) return undefined;
  return parseInteger(value);
}

function parseJsonObject(value: string): Record<string, unknown> | null {
  const trimmed = value.trim();
  if (!trimmed) return {};
  try {
    const parsed: unknown = JSON.parse(trimmed);
    return isRecord(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function firstCompletionEntry(settings: ProviderFormRoleSettings): {
  name: string;
  config: CompletionProviderModelConfig;
} | null {
  const models = settings.completion_models;
  if (!isRecord(models)) return null;
  const entry = Object.entries(models).find(([, value]) => isRecord(value));
  if (!entry) return null;
  return {
    name: entry[0],
    config: entry[1] as CompletionProviderModelConfig,
  };
}

function completionModelsRecord(
  settings: ProviderFormRoleSettings,
): Record<string, CompletionProviderModelConfig> {
  const models = settings.completion_models;
  if (!isRecord(models)) return {};
  return Object.fromEntries(
    Object.entries(models).filter(([, value]) => isRecord(value)),
  ) as Record<string, CompletionProviderModelConfig>;
}

type CompletionFormState = {
  endpoint: string;
  endpointStyle: string;
  modelName: string;
  nCtx: string;
  tokenizer: string;
  scratchpad: string;
  scratchpadPatch: string;
};

type EmbeddingFormState = {
  endpoint: string;
  endpointStyle: string;
  modelName: string;
  nCtx: string;
  tokenizer: string;
  embeddingSize: string;
  embeddingBatch: string;
  rejectionThreshold: string;
  dimensions: string;
  queryPrefix: string;
  documentPrefix: string;
};

function completionInitialState(
  settings: ProviderFormRoleSettings,
): CompletionFormState {
  const entry = firstCompletionEntry(settings);
  const config = entry?.config ?? {};
  const patch = config.scratchpad_patch;
  return {
    endpoint: stringValue(settings.completion_endpoint),
    endpointStyle:
      stringValue(settings.completion_endpoint_style) || "openai_completions",
    modelName: entry?.name ?? stringValue(config.name),
    nCtx: numberValue(config.n_ctx, DEFAULT_COMPLETION_CONTEXT),
    tokenizer: stringValue(config.tokenizer),
    scratchpad: stringValue(config.scratchpad) || "FIM-PSM",
    scratchpadPatch: isRecord(patch) ? JSON.stringify(patch, null, 2) : "{}",
  };
}

function embeddingInitialState(
  settings: ProviderFormRoleSettings,
): EmbeddingFormState {
  const config = isRecord(settings.embedding_model)
    ? (settings.embedding_model as EmbeddingProviderModelConfig)
    : {};
  return {
    endpoint: stringValue(settings.embedding_endpoint),
    endpointStyle: stringValue(settings.embedding_endpoint_style) || "openai",
    modelName: stringValue(config.name),
    nCtx: numberValue(config.n_ctx, DEFAULT_EMBEDDING_CONTEXT),
    tokenizer: stringValue(config.tokenizer),
    embeddingSize: numberValue(config.embedding_size, DEFAULT_EMBEDDING_SIZE),
    embeddingBatch: numberValue(
      config.embedding_batch,
      DEFAULT_EMBEDDING_BATCH,
    ),
    rejectionThreshold: numberValue(
      config.rejection_threshold,
      DEFAULT_REJECTION_THRESHOLD,
    ),
    dimensions: optionalNumberValue(config.dimensions),
    queryPrefix: stringValue(config.query_prefix),
    documentPrefix: stringValue(config.document_prefix),
  };
}

type RoleSeparatedModelConfigProps = {
  provider: ProviderDetailResponse;
};

export function RoleSeparatedModelConfig({
  provider,
}: RoleSeparatedModelConfigProps) {
  const settings = provider.settings as ProviderFormRoleSettings;
  const [updateProvider, { isLoading }] =
    providersApi.useUpdateProviderMutation();
  const [completion, setCompletion] = useState<CompletionFormState>(() =>
    completionInitialState(settings),
  );
  const [embedding, setEmbedding] = useState<EmbeddingFormState>(() =>
    embeddingInitialState(settings),
  );
  const [completionError, setCompletionError] = useState<string | null>(null);
  const [embeddingError, setEmbeddingError] = useState<string | null>(null);
  const [completionSaved, setCompletionSaved] = useState(false);
  const [embeddingSaved, setEmbeddingSaved] = useState(false);

  const completionOriginalName = useMemo(
    () => firstCompletionEntry(settings)?.name ?? "",
    [settings],
  );

  const saveCompletion = useCallback(async () => {
    const modelName = completion.modelName.trim();
    const nCtx = parseInteger(completion.nCtx);
    const scratchpadPatch = parseJsonObject(completion.scratchpadPatch);
    if (!modelName) {
      setCompletionError("Completion model name is required.");
      return;
    }
    if (nCtx === null) {
      setCompletionError("Completion context must be a positive integer.");
      return;
    }
    if (scratchpadPatch === null) {
      setCompletionError("Scratchpad patch must be a JSON object.");
      return;
    }

    const models = completionModelsRecord(settings);
    if (completionOriginalName && completionOriginalName !== modelName) {
      delete models[completionOriginalName];
    }
    models[modelName] = {
      ...(models[modelName] ?? {}),
      n_ctx: nCtx,
      tokenizer: completion.tokenizer.trim(),
      scratchpad: completion.scratchpad.trim(),
      scratchpad_patch: scratchpadPatch,
    };

    const response = await updateProvider({
      providerName: provider.name,
      settings: {
        ...providerIdentitySettings(provider),
        completion_endpoint: completion.endpoint.trim(),
        completion_endpoint_style: completion.endpointStyle,
        completion_models: models,
      },
    });
    if (response.error) {
      setCompletionError("Failed to save completion model configuration.");
      return;
    }
    setCompletionError(null);
    setCompletionSaved(true);
  }, [completion, completionOriginalName, provider, settings, updateProvider]);

  const saveEmbedding = useCallback(async () => {
    const modelName = embedding.modelName.trim();
    const nCtx = parseInteger(embedding.nCtx);
    const embeddingSize = parseInteger(embedding.embeddingSize);
    const embeddingBatch = parseInteger(embedding.embeddingBatch);
    const rejectionThreshold = parsePositiveNumber(
      embedding.rejectionThreshold,
    );
    const dimensions = parseOptionalInteger(embedding.dimensions);
    if (!modelName) {
      setEmbeddingError("Embedding model name is required.");
      return;
    }
    if (nCtx === null) {
      setEmbeddingError("Embedding context must be a positive integer.");
      return;
    }
    if (embeddingSize === null) {
      setEmbeddingError("Embedding size must be a positive integer.");
      return;
    }
    if (embeddingBatch === null) {
      setEmbeddingError("Embedding batch must be a positive integer.");
      return;
    }
    if (rejectionThreshold === null) {
      setEmbeddingError("Embedding threshold must be a positive number.");
      return;
    }
    if (dimensions === null) {
      setEmbeddingError("Embedding dimensions must be a positive integer.");
      return;
    }

    const response = await updateProvider({
      providerName: provider.name,
      settings: {
        ...providerIdentitySettings(provider),
        embedding_endpoint: embedding.endpoint.trim(),
        embedding_endpoint_style: embedding.endpointStyle,
        embedding_model: {
          n_ctx: nCtx,
          name: modelName,
          tokenizer: embedding.tokenizer.trim(),
          embedding_size: embeddingSize,
          embedding_batch: embeddingBatch,
          rejection_threshold: rejectionThreshold,
          ...(dimensions === undefined ? {} : { dimensions }),
          query_prefix: embedding.queryPrefix,
          document_prefix: embedding.documentPrefix,
        },
      },
    });
    if (response.error) {
      setEmbeddingError("Failed to save embedding model configuration.");
      return;
    }
    setEmbeddingError(null);
    setEmbeddingSaved(true);
  }, [embedding, provider, updateProvider]);

  return (
    <Card size="2">
      <Flex direction="column" gap="4">
        <Flex direction="column" gap="1">
          <Text size="2" weight="medium">
            Role-separated model configuration
          </Text>
          <Text size="1" color="gray">
            Configure completion and embedding roles separately from chat custom
            models. Endpoint style controls the HTTP API shape; scratchpad style
            controls FIM prompt formatting for completion models.
          </Text>
        </Flex>

        <Flex direction="column" gap="3">
          <Text size="2" weight="medium">
            Completion model
          </Text>
          <TextField.Root
            aria-label="Completion endpoint"
            placeholder="https://api.example.com/v1/completions"
            value={completion.endpoint}
            onChange={(event) =>
              setCompletion((prev) => ({
                ...prev,
                endpoint: event.target.value,
              }))
            }
          />
          <Select.Root
            value={completion.endpointStyle}
            onValueChange={(value) =>
              setCompletion((prev) => ({ ...prev, endpointStyle: value }))
            }
          >
            <Select.Trigger aria-label="Completion endpoint style" />
            <Select.Content>
              {COMPLETION_ENDPOINT_STYLES.map((style) => (
                <Select.Item key={style} value={style}>
                  {style}
                </Select.Item>
              ))}
            </Select.Content>
          </Select.Root>
          <Text size="1" color="gray">
            Use openai_completions for classic FIM completion endpoints or
            openai_chat_completions when completion is served through a chat
            API.
          </Text>
          <TextField.Root
            aria-label="Completion model name"
            placeholder="qwen2.5-coder:1.5b-base"
            value={completion.modelName}
            onChange={(event) =>
              setCompletion((prev) => ({
                ...prev,
                modelName: event.target.value,
              }))
            }
          />
          <TextField.Root
            aria-label="Completion context"
            type="number"
            value={completion.nCtx}
            onChange={(event) =>
              setCompletion((prev) => ({ ...prev, nCtx: event.target.value }))
            }
          />
          <TextField.Root
            aria-label="Completion tokenizer"
            placeholder="hf://Qwen/Qwen2.5-Coder-1.5B"
            value={completion.tokenizer}
            onChange={(event) =>
              setCompletion((prev) => ({
                ...prev,
                tokenizer: event.target.value,
              }))
            }
          />
          <TextField.Root
            aria-label="Completion scratchpad"
            placeholder="FIM-PSM"
            value={completion.scratchpad}
            onChange={(event) =>
              setCompletion((prev) => ({
                ...prev,
                scratchpad: event.target.value,
              }))
            }
          />
          <TextArea
            aria-label="Completion scratchpad patch"
            value={completion.scratchpadPatch}
            onChange={(event) =>
              setCompletion((prev) => ({
                ...prev,
                scratchpadPatch: event.target.value,
              }))
            }
          />
          {completionError && (
            <Text size="1" color="red">
              {completionError}
            </Text>
          )}
          {completionSaved && !completionError && (
            <Text size="1" color="green">
              Completion configuration saved.
            </Text>
          )}
          <Button
            size="1"
            onClick={() => void saveCompletion()}
            disabled={isLoading || provider.readonly}
          >
            Save completion model
          </Button>
        </Flex>

        <Flex direction="column" gap="3">
          <Text size="2" weight="medium">
            Embedding model
          </Text>
          <TextField.Root
            aria-label="Embedding endpoint"
            placeholder="https://api.example.com/v1/embeddings"
            value={embedding.endpoint}
            onChange={(event) =>
              setEmbedding((prev) => ({
                ...prev,
                endpoint: event.target.value,
              }))
            }
          />
          <Select.Root
            value={embedding.endpointStyle}
            onValueChange={(value) =>
              setEmbedding((prev) => ({ ...prev, endpointStyle: value }))
            }
          >
            <Select.Trigger aria-label="Embedding endpoint style" />
            <Select.Content>
              {EMBEDDING_ENDPOINT_STYLES.map((style) => (
                <Select.Item key={style} value={style}>
                  {style}
                </Select.Item>
              ))}
            </Select.Content>
          </Select.Root>
          <Text size="1" color="gray">
            OpenAI-compatible embeddings use /v1/embeddings style payloads;
            Ollama-native embeddings use Ollama's embedding API and model names.
          </Text>
          <TextField.Root
            aria-label="Embedding model name"
            placeholder="text-embedding-3-small"
            value={embedding.modelName}
            onChange={(event) =>
              setEmbedding((prev) => ({
                ...prev,
                modelName: event.target.value,
              }))
            }
          />
          <TextField.Root
            aria-label="Embedding context"
            type="number"
            value={embedding.nCtx}
            onChange={(event) =>
              setEmbedding((prev) => ({ ...prev, nCtx: event.target.value }))
            }
          />
          <TextField.Root
            aria-label="Embedding size"
            type="number"
            value={embedding.embeddingSize}
            onChange={(event) =>
              setEmbedding((prev) => ({
                ...prev,
                embeddingSize: event.target.value,
              }))
            }
          />
          <TextField.Root
            aria-label="Embedding batch"
            type="number"
            value={embedding.embeddingBatch}
            onChange={(event) =>
              setEmbedding((prev) => ({
                ...prev,
                embeddingBatch: event.target.value,
              }))
            }
          />
          <TextField.Root
            aria-label="Embedding threshold"
            type="number"
            value={embedding.rejectionThreshold}
            onChange={(event) =>
              setEmbedding((prev) => ({
                ...prev,
                rejectionThreshold: event.target.value,
              }))
            }
          />
          <TextField.Root
            aria-label="Embedding tokenizer"
            placeholder="hf://Xenova/text-embedding-3-small"
            value={embedding.tokenizer}
            onChange={(event) =>
              setEmbedding((prev) => ({
                ...prev,
                tokenizer: event.target.value,
              }))
            }
          />
          <TextField.Root
            aria-label="Embedding dimensions"
            type="number"
            placeholder="optional"
            value={embedding.dimensions}
            onChange={(event) =>
              setEmbedding((prev) => ({
                ...prev,
                dimensions: event.target.value,
              }))
            }
          />
          <TextField.Root
            aria-label="Embedding query prefix"
            value={embedding.queryPrefix}
            onChange={(event) =>
              setEmbedding((prev) => ({
                ...prev,
                queryPrefix: event.target.value,
              }))
            }
          />
          <TextField.Root
            aria-label="Embedding document prefix"
            value={embedding.documentPrefix}
            onChange={(event) =>
              setEmbedding((prev) => ({
                ...prev,
                documentPrefix: event.target.value,
              }))
            }
          />
          {embeddingError && (
            <Text size="1" color="red">
              {embeddingError}
            </Text>
          )}
          {embeddingSaved && !embeddingError && (
            <Text size="1" color="green">
              Embedding configuration saved.
            </Text>
          )}
          <Button
            size="1"
            onClick={() => void saveEmbedding()}
            disabled={isLoading || provider.readonly}
          >
            Save embedding model
          </Button>
        </Flex>
      </Flex>
    </Card>
  );
}
