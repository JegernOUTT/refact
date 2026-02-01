import { Box, Container, Flex } from "@radix-ui/themes";
import { useCopyToClipboard } from "../../hooks/useCopyToClipboard";
import React, { useCallback, useMemo, useState } from "react";
import { selectMessages } from "../../features/Chat";

import { useAppSelector } from "../../hooks";
import { isUserMessage, type UserMessage } from "../../services/refact";

import { RetryForm } from "../ChatForm";
import { DialogImage } from "../DialogImage";
import { Markdown } from "../Markdown";
import styles from "./ChatContent.module.css";
import { Reveal } from "../Reveal";
import { MessageFooter, MessageWrapper } from "./MessageFooter";

export type UserInputProps = {
  children: UserMessage["content"];
  messageIndex: number;
  messageId?: string;
  onRetry: (index: number, question: UserMessage["content"]) => void;
  onBranch?: (messageId: string) => void;
  onDelete?: (messageId: string) => void;
};

export const UserInput: React.FC<UserInputProps> = ({
  messageIndex,
  messageId,
  children,
  onRetry,
  onBranch,
  onDelete,
}) => {
  const messages = useAppSelector(selectMessages);
  const copyToClipboard = useCopyToClipboard();

  const [showTextArea, setShowTextArea] = useState(false);

  const handleCopy = useCallback(() => {
    const text =
      typeof children === "string"
        ? children
        : children
            .filter((c) => {
              if ("type" in c && c.type === "text") return true;
              if ("m_type" in c && c.m_type === "text") return true;
              return false;
            })
            .map((c) => {
              if ("text" in c) return c.text;
              if ("m_content" in c) return String(c.m_content);
              return "";
            })
            .filter(Boolean)
            .join("\n");
    copyToClipboard(text);
  }, [children, copyToClipboard]);

  const handleSubmit = useCallback(
    (value: UserMessage["content"]) => {
      onRetry(messageIndex, value);
      setShowTextArea(false);
    },
    [messageIndex, onRetry],
  );

  const handleEditClick = useCallback((event: React.MouseEvent) => {
    // Don't enter edit mode if user clicked on interactive elements
    const target = event.target as HTMLElement;
    const tagName = target.tagName.toLowerCase();

    const isInteractiveElement =
      tagName === "a" ||
      tagName === "code" ||
      tagName === "pre" ||
      tagName === "button";
    const hasInteractiveParent =
      target.closest("a") !== null ||
      target.closest("pre") !== null ||
      target.closest("button") !== null;

    if (isInteractiveElement || hasInteractiveParent) {
      return;
    }

    // Skip if user is selecting text
    const selection = window.getSelection();
    if (selection && selection.toString().length > 0) {
      return;
    }

    setShowTextArea(true);
  }, []);

  // Extract text content for rendering
  const textContent = useMemo(() => {
    if (typeof children === "string") return children;
    return children
      .filter((c) => {
        if ("type" in c && c.type === "text") return true;
        if ("m_type" in c && c.m_type === "text") return true;
        return false;
      })
      .map((c) => {
        if ("text" in c) return c.text;
        if ("m_content" in c) return String(c.m_content);
        return "";
      })
      .filter(Boolean)
      .join("\n");
  }, [children]);

  // Extract images for rendering
  const images = useMemo(() => {
    if (typeof children === "string") return [];
    return children.filter((c) => {
      if ("type" in c && c.type === "image_url") return true;
      if ("m_type" in c && c.m_type.startsWith("image/")) return true;
      return false;
    });
  }, [children]);

  const checkpointsFromMessage = useMemo(() => {
    const maybeUserMessage = messages[messageIndex];
    if (!isUserMessage(maybeUserMessage)) return null;
    return maybeUserMessage.checkpoints;
  }, [messageIndex, messages]);

  const isCompressed = useMemo(() => {
    if (typeof children !== "string") return false;
    return children.startsWith("🗜️ ");
  }, [children]);

  if (showTextArea) {
    return (
      <Container pt="1">
        <RetryForm
          onSubmit={handleSubmit}
          value={children}
          onClose={() => setShowTextArea(false)}
        />
      </Container>
    );
  }

  return (
    <MessageWrapper>
      <Container pt="1">
        <Flex justify="end">
          <Box className={styles.userInput} onClick={handleEditClick}>
            {/* Message content */}
            {isCompressed ? (
              <Reveal defaultOpen={false}>
                <Markdown canHaveInteractiveElements={false}>
                  {textContent}
                </Markdown>
              </Reveal>
            ) : (
              <>
                {/* Render markdown for text content */}
                {textContent && (
                  <Markdown canHaveInteractiveElements={true}>
                    {textContent}
                  </Markdown>
                )}

                {/* Render images - stop propagation to prevent edit mode */}
                {images.length > 0 && (
                  <Flex
                    gap="2"
                    wrap="wrap"
                    mt={textContent ? "2" : "0"}
                    onClick={(e) => e.stopPropagation()}
                  >
                    {images.map((image, index) => {
                      if ("type" in image && image.type === "image_url") {
                        return (
                          <DialogImage
                            key={`img-${index}`}
                            src={image.image_url.url}
                          />
                        );
                      }
                      if (
                        "m_type" in image &&
                        image.m_type.startsWith("image/")
                      ) {
                        const content = `data:${image.m_type};base64,${image.m_content}`;
                        return (
                          <DialogImage key={`img-${index}`} src={content} />
                        );
                      }
                      return null;
                    })}
                  </Flex>
                )}
              </>
            )}
          </Box>
        </Flex>
        <Flex justify="end">
          <MessageFooter
            messageId={messageId}
            onCopy={handleCopy}
            onBranch={onBranch}
            onDelete={onDelete}
            checkpoints={checkpointsFromMessage}
            messageIndex={messageIndex}
          />
        </Flex>
      </Container>
    </MessageWrapper>
  );
};
