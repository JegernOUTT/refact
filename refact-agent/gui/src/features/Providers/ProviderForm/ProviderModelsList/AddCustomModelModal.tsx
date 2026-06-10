import { type FC, useCallback, useEffect, useMemo, useState } from "react";

import {
  Button,
  Dialog,
  FieldError,
  FieldStack,
  FieldText,
  Switch,
} from "../../../../components/ui";
import {
  useAddCustomModelMutation,
  type AddCustomModelRequest,
  type AvailableModel,
} from "../../../../services/refact";
import styles from "./ModelCard.module.css";

export type AddCustomModelModalProps = {
  providerName: string;
  isOpen: boolean;
  onClose: () => void;
  initialModel?: AvailableModel;
  isEditingCustomModel?: boolean;
};

const DEFAULT_CONTEXT_LENGTH = "4096";

function toInputValue(value: number | null | undefined): string {
  return typeof value === "number" && Number.isFinite(value)
    ? String(value)
    : "";
}

function parseOptionalNumber(value: string): number | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;

  const parsed = Number(trimmed);
  if (!Number.isFinite(parsed)) return undefined;

  return parsed;
}

function parseReasoningEffortOptions(value: string): string[] | undefined {
  const options = value
    .split(",")
    .map((option) => option.trim())
    .filter(Boolean);

  return options.length > 0 ? options : undefined;
}

function getSaveErrorMessage(error: unknown): string {
  if (typeof error === "object" && error !== null) {
    const record = error as Record<string, unknown>;
    const data = record.data;
    if (typeof data === "object" && data !== null) {
      const dataRecord = data as Record<string, unknown>;
      if (typeof dataRecord.detail === "string") return dataRecord.detail;
      if (typeof dataRecord.error === "string") return dataRecord.error;
    }
    if (typeof data === "string") return data;
    if (typeof record.error === "string") return record.error;
    if (typeof record.message === "string") return record.message;
  }
  return "Failed to save custom model. Please try again.";
}

export const AddCustomModelModal: FC<AddCustomModelModalProps> = ({
  providerName,
  isOpen,
  onClose,
  initialModel,
  isEditingCustomModel = false,
}) => {
  const [addCustomModel, { isLoading }] = useAddCustomModelMutation();

  const [modelId, setModelId] = useState("");
  const [nCtx, setNCtx] = useState(DEFAULT_CONTEXT_LENGTH);
  const [supportsTools, setSupportsTools] = useState(false);
  const [supportsMultimodality, setSupportsMultimodality] = useState(false);
  const [supportsThinkingBudget, setSupportsThinkingBudget] = useState(false);
  const [supportsAdaptiveThinkingBudget, setSupportsAdaptiveThinkingBudget] =
    useState(false);
  const [supportsPromptCache, setSupportsPromptCache] = useState(true);
  const [tokenizer, setTokenizer] = useState("");
  const [reasoningEffortOptions, setReasoningEffortOptions] = useState("");
  const [maxOutputTokens, setMaxOutputTokens] = useState("");
  const [promptPrice, setPromptPrice] = useState("");
  const [outputPrice, setOutputPrice] = useState("");
  const [cacheReadPrice, setCacheReadPrice] = useState("");
  const [cacheCreationPrice, setCacheCreationPrice] = useState("");
  const [saveError, setSaveError] = useState<string | null>(null);

  const isEditing = Boolean(initialModel);

  const resetForm = useCallback((model?: AvailableModel) => {
    setModelId(model?.id ?? "");
    setNCtx(toInputValue(model?.n_ctx) || DEFAULT_CONTEXT_LENGTH);
    setSupportsTools(model?.supports_tools ?? false);
    setSupportsMultimodality(model?.supports_multimodality ?? false);
    setSupportsThinkingBudget(model?.supports_thinking_budget ?? false);
    setSupportsAdaptiveThinkingBudget(
      model?.supports_adaptive_thinking_budget ?? false,
    );
    setSupportsPromptCache(model?.supports_cache_control ?? true);
    setTokenizer(model?.tokenizer ?? "");
    setReasoningEffortOptions(
      model?.reasoning_effort_options?.join(", ") ?? "",
    );
    setMaxOutputTokens(toInputValue(model?.max_output_tokens));
    setPromptPrice(toInputValue(model?.pricing?.prompt));
    setOutputPrice(toInputValue(model?.pricing?.generated));
    setCacheReadPrice(toInputValue(model?.pricing?.cache_read));
    setCacheCreationPrice(toInputValue(model?.pricing?.cache_creation));
    setSaveError(null);
  }, []);

  useEffect(() => {
    if (!isOpen) return;
    resetForm(initialModel);
  }, [initialModel, isOpen, resetForm]);

  const parsedNCtx = parseOptionalNumber(nCtx);
  const parsedMaxOutputTokens = parseOptionalNumber(maxOutputTokens);
  const parsedPromptPrice = parseOptionalNumber(promptPrice);
  const parsedOutputPrice = parseOptionalNumber(outputPrice);
  const parsedCacheReadPrice = parseOptionalNumber(cacheReadPrice);
  const parsedCacheCreationPrice = parseOptionalNumber(cacheCreationPrice);

  const pricingRequested = useMemo(() => {
    return [promptPrice, outputPrice, cacheReadPrice, cacheCreationPrice].some(
      (value) => value.trim().length > 0,
    );
  }, [cacheCreationPrice, cacheReadPrice, outputPrice, promptPrice]);

  const trimmedModelId = modelId.trim();

  const isPricingValid =
    !pricingRequested ||
    (parsedPromptPrice !== undefined &&
      parsedPromptPrice >= 0 &&
      parsedOutputPrice !== undefined &&
      parsedOutputPrice >= 0 &&
      (parsedCacheReadPrice === undefined || parsedCacheReadPrice >= 0) &&
      (parsedCacheCreationPrice === undefined ||
        parsedCacheCreationPrice >= 0));

  const isValid =
    trimmedModelId.length > 0 &&
    parsedNCtx !== undefined &&
    Number.isInteger(parsedNCtx) &&
    parsedNCtx > 0 &&
    (parsedMaxOutputTokens === undefined ||
      (Number.isInteger(parsedMaxOutputTokens) && parsedMaxOutputTokens > 0)) &&
    isPricingValid;

  const handleSubmit = useCallback(async () => {
    if (!isValid) return;

    const model: AddCustomModelRequest = {
      id: trimmedModelId,
      n_ctx: parsedNCtx,
      supports_tools: supportsTools,
      supports_multimodality: supportsMultimodality,
      supports_thinking_budget: supportsThinkingBudget,
      supports_adaptive_thinking_budget: supportsAdaptiveThinkingBudget,
      supports_cache_control: supportsPromptCache,
      reasoning_effort_options:
        parseReasoningEffortOptions(reasoningEffortOptions) ?? null,
      tokenizer: tokenizer.trim() || null,
      max_output_tokens: parsedMaxOutputTokens,
      pricing: pricingRequested
        ? {
            prompt: parsedPromptPrice ?? 0,
            generated: parsedOutputPrice ?? 0,
            cache_read: parsedCacheReadPrice,
            cache_creation: parsedCacheCreationPrice,
          }
        : null,
    };

    setSaveError(null);

    try {
      await addCustomModel({ providerName, model }).unwrap();
      resetForm();
      onClose();
    } catch (error) {
      setSaveError(getSaveErrorMessage(error));
    }
  }, [
    addCustomModel,
    providerName,
    isValid,
    parsedCacheCreationPrice,
    parsedCacheReadPrice,
    parsedMaxOutputTokens,
    parsedNCtx,
    parsedOutputPrice,
    parsedPromptPrice,
    pricingRequested,
    reasoningEffortOptions,
    resetForm,
    supportsAdaptiveThinkingBudget,
    supportsPromptCache,
    supportsTools,
    supportsMultimodality,
    supportsThinkingBudget,
    tokenizer,
    trimmedModelId,
    onClose,
  ]);

  const title = isEditing
    ? isEditingCustomModel
      ? "Edit Custom Model"
      : "Edit Model Capabilities"
    : "Add Custom Model";
  const description = isEditing
    ? `Adjust the saved capability overrides for ${
        initialModel?.display_name ?? initialModel?.id ?? "this model"
      }.`
    : `Define a custom model for ${providerName}. You can set its capabilities manually.`;

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <Dialog.Content maxWidth="450px">
        <Dialog.Title>{title}</Dialog.Title>
        <Dialog.Description>{description}</Dialog.Description>

        <div className={styles.modalStack}>
          <FieldStack
            label="Model ID"
            required
            helper={
              isEditing && !isEditingCustomModel
                ? "This saves overrides for the provider/model.dev model without changing its ID."
                : undefined
            }
            control={
              <FieldText
                placeholder="e.g., my-custom-model"
                value={modelId}
                onChange={setModelId}
                disabled={isEditing}
              />
            }
          />
          <FieldStack
            label="Context Length"
            required
            control={
              <FieldText
                type="number"
                placeholder="4096"
                value={nCtx}
                onChange={setNCtx}
              />
            }
          />
          <FieldStack
            label="Max Output Tokens"
            helper="Optional"
            control={
              <FieldText
                type="number"
                placeholder="e.g., 8192"
                value={maxOutputTokens}
                onChange={setMaxOutputTokens}
              />
            }
          />

          <div className={styles.modalGroup}>
            <div className={styles.modalGroupTitle}>Capabilities</div>
            <Switch
              label="Supports Tools (function calling)"
              checked={supportsTools}
              onCheckedChange={setSupportsTools}
            />
            <Switch
              label="Supports Images/Vision"
              checked={supportsMultimodality}
              onCheckedChange={setSupportsMultimodality}
            />
            <Switch
              label="Supports Thinking Budget"
              checked={supportsThinkingBudget}
              onCheckedChange={setSupportsThinkingBudget}
            />
            <Switch
              label="Supports Adaptive Thinking Budget"
              checked={supportsAdaptiveThinkingBudget}
              onCheckedChange={setSupportsAdaptiveThinkingBudget}
            />
            <Switch
              label="Supports Prompt Caching"
              checked={supportsPromptCache}
              onCheckedChange={setSupportsPromptCache}
            />
          </div>

          <FieldStack
            label="Reasoning Effort Options"
            helper="Comma-separated values for providers that support named reasoning levels."
            control={
              <FieldText
                placeholder="low, medium, high"
                value={reasoningEffortOptions}
                onChange={setReasoningEffortOptions}
              />
            }
          />
          <FieldStack
            label="Tokenizer"
            helper="HuggingFace tokenizer path for accurate token counting."
            control={
              <FieldText
                placeholder="hf://Xenova/claude-tokenizer"
                value={tokenizer}
                onChange={setTokenizer}
              />
            }
          />

          <div className={styles.modalGroup}>
            <div className={styles.modalGroupTitle}>Pricing per 1M Tokens</div>
            <FieldText
              type="number"
              placeholder="Prompt, e.g. 1.25"
              value={promptPrice}
              onChange={setPromptPrice}
            />
            <FieldText
              type="number"
              placeholder="Output, e.g. 10"
              value={outputPrice}
              onChange={setOutputPrice}
            />
            <FieldText
              type="number"
              placeholder="Cache Read optional"
              value={cacheReadPrice}
              onChange={setCacheReadPrice}
            />
            <FieldText
              type="number"
              placeholder="Cache Creation optional"
              value={cacheCreationPrice}
              onChange={setCacheCreationPrice}
            />
            {pricingRequested && !isPricingValid ? (
              <span className={styles.noticeDanger}>
                Enter valid non-negative prompt and output prices to save
                pricing.
              </span>
            ) : null}
          </div>

          {saveError ? <FieldError>{saveError}</FieldError> : null}
        </div>

        <div className={styles.modalActions}>
          <Dialog.Close asChild>
            <Button variant="soft" disabled={isLoading}>
              Cancel
            </Button>
          </Dialog.Close>
          <Button
            variant="primary"
            onClick={() => void handleSubmit()}
            disabled={!isValid || isLoading}
          >
            {isLoading
              ? isEditing
                ? "Saving..."
                : "Adding..."
              : isEditing
                ? "Save Changes"
                : "Add Model"}
          </Button>
        </div>
      </Dialog.Content>
    </Dialog>
  );
};
