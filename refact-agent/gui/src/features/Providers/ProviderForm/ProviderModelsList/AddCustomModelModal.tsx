import { type FC, useState, useCallback } from "react";
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
} from "../../../../services/refact";

export type AddCustomModelModalProps = {
  providerName: string;
  isOpen: boolean;
  onClose: () => void;
};

export const AddCustomModelModal: FC<AddCustomModelModalProps> = ({
  providerName,
  isOpen,
  onClose,
}) => {
  const [addCustomModel, { isLoading }] = useAddCustomModelMutation();

  const [modelId, setModelId] = useState("");
  const [nCtx, setNCtx] = useState("4096");
  const [supportsTools, setSupportsTools] = useState(false);
  const [supportsMultimodality, setSupportsMultimodality] = useState(false);
  const [supportsThinkingBudget, setSupportsThinkingBudget] = useState(false);
  const [tokenizer, setTokenizer] = useState("");

  const resetForm = useCallback(() => {
    setModelId("");
    setNCtx("4096");
    setSupportsTools(false);
    setSupportsMultimodality(false);
    setSupportsThinkingBudget(false);
    setTokenizer("");
  }, []);

  const handleSubmit = useCallback(async () => {
    const model: AddCustomModelRequest = {
      id: modelId.trim(),
      n_ctx: parseInt(nCtx, 10) || 4096,
      supports_tools: supportsTools,
      supports_multimodality: supportsMultimodality,
      supports_thinking_budget: supportsThinkingBudget,
      tokenizer: tokenizer.trim() || null,
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
    modelId,
    nCtx,
    supportsTools,
    supportsMultimodality,
    supportsThinkingBudget,
    tokenizer,
    resetForm,
    onClose,
  ]);

  const isValid = modelId.trim().length > 0 && parseInt(nCtx, 10) > 0;

  return (
    <Dialog.Root open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <Dialog.Content style={{ maxWidth: 450 }}>
        <Dialog.Title>Add Custom Model</Dialog.Title>
        <Dialog.Description size="2" mb="4">
          Define a custom model for {providerName}. You&apos;ll need to specify
          its capabilities manually.
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
            {isLoading ? "Adding..." : "Add Model"}
          </Button>
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};
