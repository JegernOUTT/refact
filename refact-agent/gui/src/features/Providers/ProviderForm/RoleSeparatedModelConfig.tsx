import { useCallback, useMemo, useState } from "react";

import {
  Button,
  FieldError,
  FieldStack,
  FieldText,
  FieldTextarea,
  SaveStatus,
  Select,
} from "../../../components/ui";
import { SettingsGroup } from "../../Settings/SettingsSection";
import type {
  CompletionProviderModelConfig,
  ProviderDetailResponse,
  ProviderFormRoleSettings,
} from "../../../services/refact";
import {
  providersApi,
  providerIdentitySettings,
} from "../../../services/refact";
import styles from "./RoleSeparatedModelConfig.module.css";

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
    config: entry[1],
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
    ? settings.embedding_model
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

type EndpointStyleSelectProps = {
  ariaLabel: string;
  value: string;
  options: string[];
  onChange: (value: string) => void;
};

function EndpointStyleSelect({
  ariaLabel,
  value,
  options,
  onChange,
}: EndpointStyleSelectProps) {
  return (
    <Select value={value} onValueChange={onChange}>
      <Select.Trigger aria-label={ariaLabel} />
      <Select.Content>
        {options.map((style) => (
          <Select.Item key={style} value={style}>
            {style}
          </Select.Item>
        ))}
      </Select.Content>
    </Select>
  );
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

    let models = completionModelsRecord(settings);
    if (completionOriginalName && completionOriginalName !== modelName) {
      models = Object.fromEntries(
        Object.entries(models).filter(
          ([name]) => name !== completionOriginalName,
        ),
      );
    }
    models[modelName] = {
      ...models[modelName],
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

  const disabled = isLoading || provider.readonly;

  return (
    <section className={`${styles.root} rf-enter`}>
      <div className={styles.intro}>
        <h2 className={styles.title}>Role-separated model configuration</h2>
        <p className={styles.description}>
          Configure completion and embedding roles separately from chat custom
          models. Endpoint style controls the HTTP API shape; scratchpad style
          controls FIM prompt formatting for completion models.
        </p>
      </div>

      <SettingsGroup title="Completion model">
        <FieldStack
          label="Endpoint"
          control={
            <FieldText
              aria-label="Completion endpoint"
              placeholder="https://api.example.com/v1/completions"
              value={completion.endpoint}
              onChange={(value) =>
                setCompletion((prev) => ({ ...prev, endpoint: value }))
              }
            />
          }
        />
        <FieldStack
          label="Endpoint style"
          helper="Use openai_completions for classic FIM completion endpoints or openai_chat_completions when completion is served through a chat API."
          control={
            <EndpointStyleSelect
              ariaLabel="Completion endpoint style"
              value={completion.endpointStyle}
              options={COMPLETION_ENDPOINT_STYLES}
              onChange={(value) =>
                setCompletion((prev) => ({ ...prev, endpointStyle: value }))
              }
            />
          }
        />
        <FieldStack
          label="Model name"
          control={
            <FieldText
              aria-label="Completion model name"
              placeholder="qwen2.5-coder:1.5b-base"
              value={completion.modelName}
              onChange={(value) =>
                setCompletion((prev) => ({ ...prev, modelName: value }))
              }
            />
          }
        />
        <FieldStack
          label="Context length"
          control={
            <FieldText
              aria-label="Completion context"
              type="number"
              value={completion.nCtx}
              onChange={(value) =>
                setCompletion((prev) => ({ ...prev, nCtx: value }))
              }
            />
          }
        />
        <FieldStack
          label="Tokenizer"
          control={
            <FieldText
              aria-label="Completion tokenizer"
              placeholder="hf://Qwen/Qwen2.5-Coder-1.5B"
              value={completion.tokenizer}
              onChange={(value) =>
                setCompletion((prev) => ({ ...prev, tokenizer: value }))
              }
            />
          }
        />
        <FieldStack
          label="Scratchpad"
          control={
            <FieldText
              aria-label="Completion scratchpad"
              placeholder="FIM-PSM"
              value={completion.scratchpad}
              onChange={(value) =>
                setCompletion((prev) => ({ ...prev, scratchpad: value }))
              }
            />
          }
        />
        <FieldStack
          label="Scratchpad patch (JSON)"
          control={
            <FieldTextarea
              aria-label="Completion scratchpad patch"
              value={completion.scratchpadPatch}
              onChange={(value) =>
                setCompletion((prev) => ({ ...prev, scratchpadPatch: value }))
              }
            />
          }
        />
        {completionError ? <FieldError>{completionError}</FieldError> : null}
        <div className={styles.footer}>
          <Button
            variant="primary"
            onClick={() => void saveCompletion()}
            disabled={disabled}
          >
            Save completion model
          </Button>
          <SaveStatus
            state={completionSaved && !completionError ? "saved" : "idle"}
            label="Completion configuration saved"
          />
        </div>
      </SettingsGroup>

      <SettingsGroup title="Embedding model">
        <FieldStack
          label="Endpoint"
          control={
            <FieldText
              aria-label="Embedding endpoint"
              placeholder="https://api.example.com/v1/embeddings"
              value={embedding.endpoint}
              onChange={(value) =>
                setEmbedding((prev) => ({ ...prev, endpoint: value }))
              }
            />
          }
        />
        <FieldStack
          label="Endpoint style"
          helper="OpenAI-compatible embeddings use /v1/embeddings style payloads; Ollama-native embeddings use Ollama's embedding API and model names."
          control={
            <EndpointStyleSelect
              ariaLabel="Embedding endpoint style"
              value={embedding.endpointStyle}
              options={EMBEDDING_ENDPOINT_STYLES}
              onChange={(value) =>
                setEmbedding((prev) => ({ ...prev, endpointStyle: value }))
              }
            />
          }
        />
        <FieldStack
          label="Model name"
          control={
            <FieldText
              aria-label="Embedding model name"
              placeholder="text-embedding-3-small"
              value={embedding.modelName}
              onChange={(value) =>
                setEmbedding((prev) => ({ ...prev, modelName: value }))
              }
            />
          }
        />
        <FieldStack
          label="Context length"
          control={
            <FieldText
              aria-label="Embedding context"
              type="number"
              value={embedding.nCtx}
              onChange={(value) =>
                setEmbedding((prev) => ({ ...prev, nCtx: value }))
              }
            />
          }
        />
        <FieldStack
          label="Embedding size"
          control={
            <FieldText
              aria-label="Embedding size"
              type="number"
              value={embedding.embeddingSize}
              onChange={(value) =>
                setEmbedding((prev) => ({ ...prev, embeddingSize: value }))
              }
            />
          }
        />
        <FieldStack
          label="Embedding batch"
          control={
            <FieldText
              aria-label="Embedding batch"
              type="number"
              value={embedding.embeddingBatch}
              onChange={(value) =>
                setEmbedding((prev) => ({ ...prev, embeddingBatch: value }))
              }
            />
          }
        />
        <FieldStack
          label="Rejection threshold"
          control={
            <FieldText
              aria-label="Embedding threshold"
              type="number"
              value={embedding.rejectionThreshold}
              onChange={(value) =>
                setEmbedding((prev) => ({
                  ...prev,
                  rejectionThreshold: value,
                }))
              }
            />
          }
        />
        <FieldStack
          label="Tokenizer"
          control={
            <FieldText
              aria-label="Embedding tokenizer"
              placeholder="hf://Xenova/text-embedding-3-small"
              value={embedding.tokenizer}
              onChange={(value) =>
                setEmbedding((prev) => ({ ...prev, tokenizer: value }))
              }
            />
          }
        />
        <FieldStack
          label="Dimensions (optional)"
          control={
            <FieldText
              aria-label="Embedding dimensions"
              type="number"
              placeholder="optional"
              value={embedding.dimensions}
              onChange={(value) =>
                setEmbedding((prev) => ({ ...prev, dimensions: value }))
              }
            />
          }
        />
        <FieldStack
          label="Query prefix"
          control={
            <FieldText
              aria-label="Embedding query prefix"
              value={embedding.queryPrefix}
              onChange={(value) =>
                setEmbedding((prev) => ({ ...prev, queryPrefix: value }))
              }
            />
          }
        />
        <FieldStack
          label="Document prefix"
          control={
            <FieldText
              aria-label="Embedding document prefix"
              value={embedding.documentPrefix}
              onChange={(value) =>
                setEmbedding((prev) => ({ ...prev, documentPrefix: value }))
              }
            />
          }
        />
        {embeddingError ? <FieldError>{embeddingError}</FieldError> : null}
        <div className={styles.footer}>
          <Button
            variant="primary"
            onClick={() => void saveEmbedding()}
            disabled={disabled}
          >
            Save embedding model
          </Button>
          <SaveStatus
            state={embeddingSaved && !embeddingError ? "saved" : "idle"}
            label="Embedding configuration saved"
          />
        </div>
      </SettingsGroup>
    </section>
  );
}
