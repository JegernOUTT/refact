import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Check, ChevronDown, Plus, X } from "lucide-react";
import { FileRejection, useDropzone } from "react-dropzone";
import classNames from "classnames";

import { TextArea } from "../TextArea";
import { useAppSelector, useCapsForToolUse } from "../../hooks";
import {
  ProcessedUserMessageContentWithImages,
  UserImage,
  UserMessage,
} from "../../services/refact";
import { useAttachedImages } from "../../hooks/useAttachedImages";
import {
  selectIsStreamingById,
  selectIsWaitingById,
  useThreadId,
} from "../../features/Chat/Thread";
import { enrichAndGroupModels } from "../../utils/enrichModels";
import { DialogImage } from "../DialogImage";
import {
  Button,
  IconButton,
  ModelSelector,
  Popover,
  type ModelOption,
  type ModelSelectorBadge,
  type ModelSelectorGroup,
} from "../ui";
import {
  formatContextWindow,
  formatPricing,
} from "../../features/Providers/ProviderForm/ProviderModelsList/utils/groupModelsWithPricing";
import styles from "./RetryForm.module.css";

function getTextFromUserMessage(messages: UserMessage["content"]): string {
  if (typeof messages === "string") return messages;
  return messages.reduce<string>((acc, message) => {
    if ("m_type" in message && message.m_type === "text") {
      return acc + message.m_content;
    }
    if ("type" in message && message.type === "text") return acc + message.text;
    return acc;
  }, "");
}

function getImageFromUserMessage(
  messages: UserMessage["content"],
): (UserImage | ProcessedUserMessageContentWithImages)[] {
  if (typeof messages === "string") return [];

  return messages.reduce<(UserImage | ProcessedUserMessageContentWithImages)[]>(
    (acc, message) => {
      if ("m_type" in message && message.m_type.startsWith("image/")) {
        return [...acc, message];
      }
      if ("type" in message && message.type === "image_url") {
        return [...acc, message];
      }
      return acc;
    },
    [],
  );
}

function getImageContent(
  image: UserImage | ProcessedUserMessageContentWithImages,
) {
  if ("type" in image) return image.image_url.url;
  return `data:${image.m_type};base64,${image.m_content}`;
}

export const RetryForm: React.FC<{
  value: UserMessage["content"];
  onSubmit: (value: UserMessage["content"]) => void;
  onClose: () => void;
}> = (props) => {
  const { isMultimodalitySupportedForCurrentModel } = useCapsForToolUse();
  const inputText = getTextFromUserMessage(props.value);
  const inputImages = getImageFromUserMessage(props.value);
  const [textValue, onChangeTextValue] = useState(inputText);
  const [imageValue, onChangeImageValue] = useState(inputImages);
  const chatId = useThreadId();
  const isStreaming = useAppSelector((state) =>
    selectIsStreamingById(state, chatId),
  );
  const isWaiting = useAppSelector((state) => selectIsWaitingById(state, chatId));
  const formRef = useRef<HTMLDivElement>(null);

  const disableInput = useMemo(
    () => isStreaming || isWaiting,
    [isStreaming, isWaiting],
  );

  const addImage = useCallback((image: UserImage) => {
    onChangeImageValue((prev) => {
      return [...prev, image];
    });
  }, []);

  const closeAndReset = useCallback(() => {
    onChangeImageValue(inputImages);
    onChangeTextValue(inputText);
    props.onClose();
  }, [inputImages, inputText, props]);

  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      const target = event.target as Node;
      if (
        target instanceof Element &&
        target.closest(`.${styles.modelContent}`)
      ) {
        return;
      }
      if (formRef.current && !formRef.current.contains(target)) {
        closeAndReset();
      }
    };

    document.addEventListener("mousedown", handleClickOutside);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, [closeAndReset]);

  const handleRetry = useCallback(() => {
    const trimmedText = textValue.trim();
    if (imageValue.length === 0 && trimmedText.length > 0) {
      props.onSubmit(trimmedText);
    } else if (trimmedText.length > 0 || imageValue.length > 0) {
      const content: (
        | { type: "text"; text: string }
        | UserImage
        | ProcessedUserMessageContentWithImages
      )[] = [];
      if (trimmedText.length > 0) {
        content.push({ type: "text" as const, text: trimmedText });
      }
      content.push(...imageValue);
      props.onSubmit(
        content.length === 1 && trimmedText ? trimmedText : content,
      );
    }
  }, [textValue, imageValue, props]);

  const handleOnKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (event.nativeEvent.isComposing) {
        return;
      }

      if (event.key === "Escape") {
        event.preventDefault();
        closeAndReset();
        return;
      }

      if (event.key === "Enter" && !event.shiftKey) {
        event.preventDefault();
        if (
          !disableInput &&
          (textValue.trim().length > 0 || imageValue.length > 0)
        ) {
          handleRetry();
        }
      }
    },
    [closeAndReset, disableInput, textValue, imageValue, handleRetry],
  );

  const handleRemove = useCallback((index: number) => {
    onChangeImageValue((prev) => {
      return prev.filter((_, i) => i !== index);
    });
  }, []);

  return (
    <div ref={formRef} className={styles.root}>
      <form
        className={styles.form}
        onSubmit={(event) => {
          event.preventDefault();
          handleRetry();
        }}
      >
        {imageValue.length > 0 && (
          <div className={styles.attachments}>
            {imageValue.map((image, index) => {
              return (
                <RetryImage
                  key={`retry-user-image-${index}`}
                  image={getImageContent(image)}
                  onRemove={() => handleRemove(index)}
                />
              );
            })}
          </div>
        )}

        <div className={styles.textareaWrapper}>
          <TextArea
            value={textValue}
            onChange={(event) => onChangeTextValue(event.target.value)}
            onKeyDown={handleOnKeyDown}
            autoFocus
          />
        </div>

        <div className={styles.controls}>
          <Button
            leftIcon={X}
            size="sm"
            type="button"
            variant="ghost"
            onClick={closeAndReset}
          >
            Cancel
          </Button>

          <span className={styles.controlsSpacer} />

          <RetryModelSelector disabled={disableInput} />
          {isMultimodalitySupportedForCurrentModel && (
            <RetryDropzone addImage={addImage} />
          )}
          <Button
            disabled={
              disableInput ||
              (textValue.trim().length === 0 && imageValue.length === 0)
            }
            leftIcon={Check}
            size="sm"
            type="submit"
            variant="primary"
          >
            Submit
          </Button>
        </div>
      </form>
    </div>
  );
};

const RetryDropzone: React.FC<{
  addImage: (image: UserImage) => void;
}> = ({ addImage }) => {
  const { setError, setWarning } = useAttachedImages();

  const onDrop = useCallback(
    (acceptedFiles: File[], fileRejections: FileRejection[]) => {
      acceptedFiles.forEach((file) => {
        const reader = new FileReader();
        reader.onabort = () =>
          setWarning(`file ${file.name} reading was aborted`);
        reader.onerror = () => setError(`file ${file.name} reading has failed`);
        reader.onload = () => {
          if (typeof reader.result === "string") {
            const image: UserImage = {
              type: "image_url",
              image_url: { url: reader.result },
            };
            addImage(image);
          }
        };
        reader.readAsDataURL(file);
      });

      if (fileRejections.length) {
        const rejectedFileMessage = fileRejections.map((file) => {
          const err = file.errors.reduce<string>((acc, cur) => {
            return acc + `${cur.code} ${cur.message}\n`;
          }, "");
          return `Could not attach ${file.file.name}: ${err}`;
        });
        setError(rejectedFileMessage.join("\n"));
      }
    },
    [addImage, setError, setWarning],
  );

  const { getRootProps, getInputProps, open } = useDropzone({
    onDrop,
    disabled: false,
    noClick: true,
    noKeyboard: true,
    accept: {
      "image/*": [],
    },
  });

  return (
    <div {...getRootProps()} className={styles.dropzoneRoot}>
      <input {...getInputProps()} hidden />
      <Button
        leftIcon={Plus}
        size="sm"
        type="button"
        variant="ghost"
        onClick={(event) => {
          event.preventDefault();
          event.stopPropagation();
          open();
        }}
      >
        Add image
      </Button>
    </div>
  );
};

const RetryImage: React.FC<{ image: string; onRemove: () => void }> = ({
  image,
  onRemove,
}) => {
  return (
    <span className={styles.imageFrame}>
      <DialogImage src={image} size="5" />
      <IconButton
        aria-label="Remove image"
        className={styles.removeImageButton}
        icon={X}
        size="sm"
        type="button"
        variant="soft"
        onClick={(event) => {
          event.preventDefault();
          event.stopPropagation();
          onRemove();
        }}
      />
    </span>
  );
};

const RetryModelSelector: React.FC<{ disabled?: boolean }> = ({ disabled }) => {
  const caps = useCapsForToolUse();
  const [isOpen, setIsOpen] = useState(false);

  const groupedModels = useMemo(() => {
    return enrichAndGroupModels(caps.usableModelsForPlan, caps.data);
  }, [caps.usableModelsForPlan, caps.data]);

  const groups = useMemo<ModelSelectorGroup[]>(() => {
    return groupedModels.map((group) => ({
      id: group.provider,
      label: group.displayName,
    }));
  }, [groupedModels]);

  const models = useMemo<ModelOption[]>(() => {
    return groupedModels.flatMap((group) =>
      group.models.map((model) => {
        const badges: ModelSelectorBadge[] = [];
        if (model.isDefault) badges.push("default");
        if (model.isThinking) badges.push("reasoning");
        if (model.isLight) badges.push("light");
        if (model.isBuddy) badges.push("buddy");
        if (model.isTaskPlannerAgent) badges.push("task-agent");
        if (model.isChat2) badges.push("chat2");

        const pricingParts = model.pricing
          ? formatPricing(model.pricing, true).split("/")
          : null;

        return {
          value: model.value,
          displayName: model.value,
          group: group.provider,
          disabled: model.disabled,
          pricing: pricingParts
            ? {
                prompt: pricingParts[0],
                output: pricingParts[1],
              }
            : undefined,
          contextWindow: model.nCtx
            ? `${formatContextWindow(model.nCtx)} ctx`
            : undefined,
          badges,
          capabilities: model.capabilities ? (
            <span className={styles.capabilities}>
              {model.capabilities.supportsTools ? <span>Tools</span> : null}
              {model.capabilities.supportsMultimodality ? (
                <span>Vision</span>
              ) : null}
              {model.capabilities.supportsAgent ? <span>Agent</span> : null}
            </span>
          ) : undefined,
        };
      }),
    );
  }, [groupedModels]);

  const currentModelName = caps.currentModel || "Select model";

  const handleModelSelect = useCallback(
    (modelValue: string) => {
      caps.setCapModel(modelValue);
      setIsOpen(false);
    },
    [caps],
  );

  if (caps.loading) {
    return null;
  }

  return (
    <Popover open={isOpen} onOpenChange={setIsOpen} responsive={false}>
      <Popover.Trigger asChild>
        <Button
          className={classNames(styles.modelSelector, styles.modelTrigger)}
          disabled={disabled}
          rightIcon={ChevronDown}
          type="button"
          variant="soft"
        >
          <span className={styles.modelTriggerText}>{currentModelName}</span>
        </Button>
      </Popover.Trigger>
      <Popover.Content
        align="end"
        className={styles.modelContent}
        maxHeight="min(520px, calc(100dvh - var(--rf-space-6)))"
        maxWidth="420px"
        side="top"
        sideOffset={8}
      >
        <ModelSelector
          disabled={disabled}
          groups={groups}
          models={models}
          value={caps.currentModel}
          variant="inline"
          onSelect={handleModelSelect}
        />
      </Popover.Content>
    </Popover>
  );
};
