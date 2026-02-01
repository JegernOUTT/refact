import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Button, Flex, Box, IconButton } from "@radix-ui/themes";
import { FileRejection, useDropzone } from "react-dropzone";
import { TextArea } from "../TextArea";
import { useAppSelector, useCapsForToolUse } from "../../hooks";
import {
  ProcessedUserMessageContentWithImages,
  UserImage,
  UserMessage,
} from "../../services/refact";
import { Cross2Icon, CheckIcon, PlusIcon } from "@radix-ui/react-icons";
import { useAttachedImages } from "../../hooks/useAttachedImages";
import { selectIsStreaming, selectIsWaiting } from "../../features/Chat";
import styles from "./ChatForm.module.css";
import classNames from "classnames";
import { DialogImage } from "../DialogImage";

function getTextFromUserMessage(messages: UserMessage["content"]): string {
  if (typeof messages === "string") return messages;
  return messages.reduce<string>((acc, message) => {
    if ("m_type" in message && message.m_type === "text")
      return acc + message.m_content;
    if ("type" in message && message.type === "text") return acc + message.text;
    return acc;
  }, "");
}

function getImageFromUserMessage(
  messages: UserMessage["content"],
): (UserImage | ProcessedUserMessageContentWithImages)[] {
  if (typeof messages === "string") return [];

  const images = messages.reduce<
    (UserImage | ProcessedUserMessageContentWithImages)[]
  >((acc, message) => {
    if ("m_type" in message && message.m_type.startsWith("image/"))
      return [...acc, message];
    if ("type" in message && message.type === "image_url")
      return [...acc, message];
    return acc;
  }, []);

  return images;
}

function getImageContent(
  image: UserImage | ProcessedUserMessageContentWithImages,
) {
  if ("type" in image) return image.image_url.url;
  const base64 = `data:${image.m_type};base64,${image.m_content}`;
  return base64;
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
  const isStreaming = useAppSelector(selectIsStreaming);
  const isWaiting = useAppSelector(selectIsWaiting);
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

  // Click outside to cancel edit
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (formRef.current && !formRef.current.contains(event.target as Node)) {
        closeAndReset();
      }
    };

    // Use mousedown to catch the click before focus changes
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
      // Don't handle during IME composition
      if (event.nativeEvent.isComposing) {
        return;
      }

      // Escape: cancel and close
      if (event.key === "Escape") {
        event.preventDefault();
        closeAndReset();
        return;
      }

      // Enter without Shift: submit
      if (event.key === "Enter" && !event.shiftKey) {
        event.preventDefault();
        if (
          !disableInput &&
          (textValue.trim().length > 0 || imageValue.length > 0)
        ) {
          handleRetry();
        }
        return;
      }

      // Shift+Enter: allow newline (default behavior, no preventDefault)
    },
    [closeAndReset, disableInput, textValue, imageValue, handleRetry],
  );

  const handleRemove = useCallback((index: number) => {
    onChangeImageValue((prev) => {
      return prev.filter((_, i) => i !== index);
    });
  }, []);

  return (
    <Box
      ref={formRef}
      className={classNames(styles.chatForm, styles.chatFormCompact)}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          handleRetry();
        }}
      >
        {/* Attachments at top */}
        {imageValue.length > 0 && (
          <Flex
            px="3"
            py="2"
            wrap="wrap"
            direction="row"
            align="center"
            gap="2"
          >
            {imageValue.map((image, index) => {
              return (
                <RetryImage
                  key={`retry-user-image-${index}`}
                  image={getImageContent(image)}
                  onRemove={() => handleRemove(index)}
                />
              );
            })}
          </Flex>
        )}

        {/* TextArea */}
        <Box className={styles.textareaWrapper}>
          <TextArea
            value={textValue}
            onChange={(event) => onChangeTextValue(event.target.value)}
            onKeyDown={handleOnKeyDown}
            autoFocus
            style={{ boxShadow: "none", outline: "none" }}
          />
        </Box>

        {/* Bottom controls */}
        <Flex align="center" gap="2" py="2" px="3">
          <Button
            variant="ghost"
            color="gray"
            size="1"
            type="button"
            onClick={closeAndReset}
          >
            <Cross2Icon width={14} height={14} />
            Cancel
          </Button>

          <Box flexGrow="1" />

          {isMultimodalitySupportedForCurrentModel && (
            <RetryDropzone addImage={addImage} />
          )}
          <Button
            variant="solid"
            size="1"
            type="submit"
            disabled={
              disableInput ||
              (textValue.trim().length === 0 && imageValue.length === 0)
            }
          >
            <CheckIcon width={14} height={14} />
            Submit
          </Button>
        </Flex>
      </form>
    </Box>
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
    <div {...getRootProps()} style={{ display: "flex", alignItems: "center" }}>
      <input {...getInputProps()} style={{ display: "none" }} />
      <Button
        size="1"
        variant="ghost"
        color="gray"
        type="button"
        onClick={(event) => {
          event.preventDefault();
          event.stopPropagation();
          open();
        }}
      >
        <PlusIcon width={14} height={14} />
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
    <Box position="relative" style={{ display: "inline-block" }}>
      <DialogImage src={image} size="5" />
      <IconButton
        variant="solid"
        color="gray"
        size="1"
        type="button"
        onClick={(event) => {
          event.preventDefault();
          event.stopPropagation();
          onRemove();
        }}
        style={{
          position: "absolute",
          right: -6,
          top: -6,
          width: 18,
          height: 18,
          padding: 0,
          borderRadius: "50%",
        }}
      >
        <Cross2Icon width={10} height={10} />
      </IconButton>
    </Box>
  );
};
