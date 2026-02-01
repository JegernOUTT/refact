import { Pencil2Icon } from "@radix-ui/react-icons";
import { Button, Container, Flex, IconButton, Text } from "@radix-ui/themes";
import React, { useCallback, useMemo, useState } from "react";
import { selectMessages } from "../../features/Chat";
import { CheckpointButton } from "../../features/Checkpoints";
import { useAppSelector, useEventsBusForIDE } from "../../hooks";
import { selectHost } from "../../features/Config/configSlice";
import {
  isUserMessage,
  ProcessedUserMessageContentWithImages,
  UserMessageContentWithImage,
  type UserMessage,
} from "../../services/refact";
import {
  parseLine,
  tokenToChipInfo,
  getAtCommandTokens,
} from "../../utils/atCommands";
import { AtCommandChip } from "../AtCommands";

import { RetryForm } from "../ChatForm";
import { DialogImage } from "../DialogImage";
import { Markdown } from "../Markdown";
import styles from "./ChatContent.module.css";
import { Reveal } from "../Reveal";

export type UserInputProps = {
  children: UserMessage["content"];
  messageIndex: number;
  // maybe add images argument ?
  onRetry: (index: number, question: UserMessage["content"]) => void;
  // disableRetry?: boolean;
};

export const UserInput: React.FC<UserInputProps> = ({
  messageIndex,
  children,
  onRetry,
}) => {
  const messages = useAppSelector(selectMessages);
  const host = useAppSelector(selectHost);
  const { queryPathThenOpenFile } = useEventsBusForIDE();

  const [showTextArea, setShowTextArea] = useState(false);
  const [isEditButtonVisible, setIsEditButtonVisible] = useState(false);

  const handleSubmit = useCallback(
    (value: UserMessage["content"]) => {
      onRetry(messageIndex, value);
      setShowTextArea(false);
    },
    [messageIndex, onRetry],
  );

  const handleShowTextArea = useCallback(
    (value: boolean) => {
      setShowTextArea(value);
      if (isEditButtonVisible) {
        setIsEditButtonVisible(false);
      }
    },
    [isEditButtonVisible],
  );

  const elements = process(children, host, queryPathThenOpenFile);
  const isString = typeof children === "string";
  const linesLength = isString ? children.split("\n").length : Infinity;

  const checkpointsFromMessage = useMemo(() => {
    const maybeUserMessage = messages[messageIndex];
    if (!isUserMessage(maybeUserMessage)) return null;
    return maybeUserMessage.checkpoints;
  }, [messageIndex, messages]);

  const isCompressed = useMemo(() => {
    if (typeof children !== "string") return false;
    return children.startsWith("🗜️ ");
  }, [children]);

  return (
    <Container position="relative" pt="1">
      {isCompressed ? (
        <Reveal defaultOpen={false}>
          <Flex direction="row" my="1" className={styles.userInput}>
            {elements}
          </Flex>
        </Reveal>
      ) : showTextArea ? (
        <RetryForm
          onSubmit={handleSubmit}
          // TODO
          // value={children}
          value={children}
          onClose={() => handleShowTextArea(false)}
        />
      ) : (
        <Flex
          direction="row"
          // checking for the length of the lines to determine the position of the edit button
          gap={linesLength <= 2 ? "2" : "1"}
          // TODO: what is it's a really long sentence or word with out new lines?
          align={linesLength <= 2 ? "center" : "end"}
          my="1"
          onMouseEnter={() => setIsEditButtonVisible(true)}
          onMouseLeave={() => setIsEditButtonVisible(false)}
        >
          <Button
            // ref={ref}
            variant="soft"
            size="4"
            className={styles.userInput}
            // TODO: should this work?
            // onClick={() => handleShowTextArea(true)}
            asChild
          >
            <div>{elements}</div>
          </Button>
          <Flex
            direction={linesLength <= 3 ? "row" : "column"}
            gap="1"
            style={{
              opacity: isEditButtonVisible ? 1 : 0,
              visibility: isEditButtonVisible ? "visible" : "hidden",
              transition: "opacity 0.15s, visibility 0.15s",
            }}
          >
            {checkpointsFromMessage && checkpointsFromMessage.length > 0 && (
              <CheckpointButton
                checkpoints={checkpointsFromMessage}
                messageIndex={messageIndex}
              />
            )}
            <IconButton
              title="Edit message"
              variant="soft"
              size={"2"}
              onClick={() => handleShowTextArea(true)}
            >
              <Pencil2Icon width={15} height={15} />
            </IconButton>
          </Flex>
        </Flex>
      )}
    </Container>
  );
};

function process(
  items: UserInputProps["children"],
  host: string,
  onOpenFile: (file: { file_path: string; line?: number }) => Promise<void>,
) {
  if (typeof items !== "string") {
    return processUserInputArray(items, host, onOpenFile);
  }
  return processLines(items.split("\n"), host, onOpenFile);
}

function renderLineWithChips(
  line: string,
  key: string,
  host: string,
  onOpenFile: (file: { file_path: string; line?: number }) => Promise<void>,
  allAtTokens: ReturnType<typeof getAtCommandTokens>,
): JSX.Element {
  const parsed = parseLine(line);
  const hasAtCommands = parsed.tokens.some((t) => t.kind === "at");

  if (!hasAtCommands) {
    return (
      <Text
        size="2"
        as="div"
        key={key}
        wrap="balance"
        className={styles.break_word}
      >
        {line}
      </Text>
    );
  }

  const elements = parsed.tokens.map((token, idx) => {
    if (token.kind === "text") {
      return <span key={`${key}-text-${idx}`}>{token.text}</span>;
    }

    const chip = tokenToChipInfo(token, host === "web", allAtTokens);

    const handleClick = () => {
      if (token.type === "file" && token.arg) {
        void onOpenFile({
          file_path: token.arg,
          line: token.lineRange?.line1,
        });
      } else if (token.type === "web" && token.arg) {
        window.open(
          token.arg.startsWith("http") ? token.arg : `https://${token.arg}`,
          "_blank",
          "noopener,noreferrer",
        );
      }
    };

    return (
      <AtCommandChip
        key={`${key}-chip-${idx}`}
        chip={chip}
        onClick={handleClick}
      />
    );
  });

  return (
    <Text
      size="2"
      as="div"
      key={key}
      wrap="balance"
      className={styles.break_word}
    >
      {elements}
    </Text>
  );
}

function processLines(
  lines: string[],
  host: string,
  onOpenFile: (file: { file_path: string; line?: number }) => Promise<void>,
): JSX.Element[] {
  const allParsedLines = lines
    .filter((line) => !line.trimStart().startsWith("```"))
    .map((line) => parseLine(line));
  const allAtTokens = getAtCommandTokens(allParsedLines);

  const result: JSX.Element[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];
    const key = `line-${result.length + 1}`;

    if (!line.trimStart().startsWith("```")) {
      result.push(
        renderLineWithChips(line, key, host, onOpenFile, allAtTokens),
      );
      i++;
      continue;
    }

    let endIndex = -1;
    for (let j = i + 1; j < lines.length; j++) {
      if (lines[j].trimStart().startsWith("```")) {
        endIndex = j;
        break;
      }
    }

    if (endIndex === -1) {
      result.push(
        renderLineWithChips(line, key, host, onOpenFile, allAtTokens),
      );
      i++;
      continue;
    }

    const codeLines: string[] = [];
    for (let j = i; j <= endIndex; j++) {
      codeLines.push(lines[j]);
    }
    result.push(<Markdown key={key}>{codeLines.join("\n")}</Markdown>);
    i = endIndex + 1;
  }

  return result;
}

function isUserContentImage(
  item: UserMessageContentWithImage | ProcessedUserMessageContentWithImages,
) {
  return (
    ("m_type" in item && item.m_type.startsWith("image/")) ||
    ("type" in item && item.type === "image_url")
  );
}

function processUserInputArray(
  items: (
    | UserMessageContentWithImage
    | ProcessedUserMessageContentWithImages
  )[],
  host: string,
  onOpenFile: (file: { file_path: string; line?: number }) => Promise<void>,
): JSX.Element[] {
  const result: JSX.Element[] = [];
  let i = 0;

  while (i < items.length) {
    const head = items[i];

    if ("type" in head && head.type === "text") {
      const processedLines = processLines(
        head.text.split("\n"),
        host,
        onOpenFile,
      );
      for (const el of processedLines) result.push(el);
      i++;
      continue;
    }

    if ("m_type" in head && head.m_type === "text") {
      const processedLines = processLines(
        head.m_content.split("\n"),
        host,
        onOpenFile,
      );
      for (const el of processedLines) result.push(el);
      i++;
      continue;
    }

    if (!isUserContentImage(head)) {
      i++;
      continue;
    }

    const images: typeof items = [head];
    let j = i + 1;
    while (j < items.length && isUserContentImage(items[j])) {
      images.push(items[j]);
      j++;
    }

    result.push(
      <Flex
        key={`user-image-images-${result.length}`}
        gap="2"
        wrap="wrap"
        my="2"
      >
        {images.map((image, index) => {
          if ("type" in image && image.type === "image_url") {
            const key = `user-input${result.length}-${image.type}-${index}`;
            const content = image.image_url.url;
            return <DialogImage src={content} key={key} />;
          }
          if ("m_type" in image && image.m_type.startsWith("image/")) {
            const key = `user-input${result.length}-${image.m_type}-${index}`;
            const content = `data:${image.m_type};base64,${image.m_content}`;
            return <DialogImage src={content} key={key} />;
          }
          return null;
        })}
      </Flex>,
    );
    i = j;
  }

  return result;
}
