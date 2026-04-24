import { type FC, useCallback, useEffect, useMemo, useState } from "react";
import {
  Button,
  Checkbox,
  Dialog,
  Flex,
  Text,
  TextField,
} from "@radix-ui/themes";

import {
  useAddCustomModelMutation,
  type AddCustomModelRequest,
  type AvailableModel,
} from "../../../../services/refact";

export type AddCustomModelModalProps = {
  providerName: string;
  isOpen: boolean;
  onClose: () => void;
  initialModel?: AvailableModel;
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

export const AddCustomModelModal: FC<AddCustomModelModalProps> = ({
  providerName,
  isOpen,
  onClose,
  initialModel,
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

    try {
      await addCustomModel({ providerName, model }).unwrap();
      resetForm();
      onClose();
    } catch (e) {
      // eslint-disable-next-line no-console
      console.error("Failed to add custom model:", e);
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

  return (
    <Dialog.Root open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <Dialog.Content style={{ maxWidth: 450 }}>
        <Dialog.Title>
          {isEditing ? "Edit Custom Model" : "Add Custom Model"}
        </Dialog.Title>
        <Dialog.Description size="2" mb="4">
          {isEditing
            ? `Adjust the saved overrides for ${
                initialModel?.display_name ?? initialModel?.id ?? "this model"
              }.`
            : `Define a custom model for ${providerName}. You can set its capabilities manually.`}
        </Dialog.Description>

        <Flex direction="column" gap="3">
          <Flex direction="column" gap="1">
            <Text as="label" size="2" weight="medium">
              Model ID *
            </Text>
            <TextField.Root
              placeholder="e.g., my-custom-model"
              value={modelId}
              onChange={(e) => setModelId(e.target.value)}
              disabled={isEditing}
            />
          </Flex>

          <Flex direction="column" gap="1">
            <Text as="label" size="2" weight="medium">
              Context Length *
            </Text>
            <TextField.Root
              type="number"
              placeholder="4096"
              value={nCtx}
              onChange={(e) => setNCtx(e.target.value)}
            />
          </Flex>

          <Flex direction="column" gap="1">
            <Text as="label" size="2" weight="medium">
              Max Output Tokens (optional)
            </Text>
            <TextField.Root
              type="number"
              placeholder="e.g., 8192"
              value={maxOutputTokens}
              onChange={(e) => setMaxOutputTokens(e.target.value)}
            />
          </Flex>

          <Flex direction="column" gap="2">
            <Text as="label" size="2" weight="medium">
              Capabilities
            </Text>

            <Flex align="center" gap="2">
              <Checkbox
                id="supports_tools"
                checked={supportsTools}
                onCheckedChange={(checked) =>
                  setSupportsTools(checked === true)
                }
              />
              <Text as="label" htmlFor="supports_tools" size="2">
                Supports Tools (function calling)
              </Text>
            </Flex>

            <Flex align="center" gap="2">
              <Checkbox
                id="supports_multimodality"
                checked={supportsMultimodality}
                onCheckedChange={(checked) =>
                  setSupportsMultimodality(checked === true)
                }
              />
              <Text as="label" htmlFor="supports_multimodality" size="2">
                Supports Images/Vision
              </Text>
            </Flex>

            <Flex align="center" gap="2">
              <Checkbox
                id="supports_thinking_budget"
                checked={supportsThinkingBudget}
                onCheckedChange={(checked) =>
                  setSupportsThinkingBudget(checked === true)
                }
              />
              <Text as="label" htmlFor="supports_thinking_budget" size="2">
                Supports Thinking Budget
              </Text>
            </Flex>

            <Flex align="center" gap="2">
              <Checkbox
                id="supports_adaptive_thinking_budget"
                checked={supportsAdaptiveThinkingBudget}
                onCheckedChange={(checked) =>
                  setSupportsAdaptiveThinkingBudget(checked === true)
                }
              />
              <Text
                as="label"
                htmlFor="supports_adaptive_thinking_budget"
                size="2"
              >
                Supports Adaptive Thinking Budget
              </Text>
            </Flex>

            <Flex align="center" gap="2">
              <Checkbox
                id="supports_cache_control"
                checked={supportsPromptCache}
                onCheckedChange={(checked) =>
                  setSupportsPromptCache(checked === true)
                }
              />
              <Text as="label" htmlFor="supports_cache_control" size="2">
                Supports Prompt Caching
              </Text>
            </Flex>
          </Flex>

          <Flex direction="column" gap="1">
            <Text as="label" size="2" weight="medium">
              Reasoning Effort Options (optional)
            </Text>
            <TextField.Root
              placeholder="low, medium, high"
              value={reasoningEffortOptions}
              onChange={(e) => setReasoningEffortOptions(e.target.value)}
            />
            <Text as="span" size="1" color="gray">
              Comma-separated values for providers that support named reasoning
              levels.
            </Text>
          </Flex>

          <Flex direction="column" gap="1">
            <Text as="label" size="2" weight="medium">
              Tokenizer (optional)
            </Text>
            <TextField.Root
              placeholder="hf://Xenova/claude-tokenizer"
              value={tokenizer}
              onChange={(e) => setTokenizer(e.target.value)}
            />
            <Text as="span" size="1" color="gray">
              HuggingFace tokenizer path for accurate token counting
            </Text>
          </Flex>

          <Flex direction="column" gap="2">
            <Text as="label" size="2" weight="medium">
              Pricing per 1M Tokens (optional)
            </Text>

            <Flex direction="column" gap="1">
              <Text as="label" size="1" color="gray">
                Prompt
              </Text>
              <TextField.Root
                type="number"
                placeholder="e.g., 1.25"
                value={promptPrice}
                onChange={(e) => setPromptPrice(e.target.value)}
              />
            </Flex>

            <Flex direction="column" gap="1">
              <Text as="label" size="1" color="gray">
                Output
              </Text>
              <TextField.Root
                type="number"
                placeholder="e.g., 10"
                value={outputPrice}
                onChange={(e) => setOutputPrice(e.target.value)}
              />
            </Flex>

            <Flex direction="column" gap="1">
              <Text as="label" size="1" color="gray">
                Cache Read
              </Text>
              <TextField.Root
                type="number"
                placeholder="optional"
                value={cacheReadPrice}
                onChange={(e) => setCacheReadPrice(e.target.value)}
              />
            </Flex>

            <Flex direction="column" gap="1">
              <Text as="label" size="1" color="gray">
                Cache Creation
              </Text>
              <TextField.Root
                type="number"
                placeholder="optional"
                value={cacheCreationPrice}
                onChange={(e) => setCacheCreationPrice(e.target.value)}
              />
            </Flex>

            {pricingRequested && !isPricingValid && (
              <Text as="span" size="1" color="red">
                Enter valid non-negative prompt and output prices to save
                pricing.
              </Text>
            )}
          </Flex>
        </Flex>

        <Flex gap="3" mt="4" justify="end">
          <Dialog.Close>
            <Button variant="soft" color="gray">
              Cancel
            </Button>
          </Dialog.Close>
          <Button
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
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};
