import React, { useCallback, useMemo } from "react";
import { Markdown } from "../Markdown";

import { Box, Flex, Text, Card } from "@radix-ui/themes";
import { Link } from "../Link";
import {
  ChatContextFile,
  DiffChunk,
  ThinkingBlock,
  ToolCall,
  Usage,
  WebSearchCitation,
} from "../../services/refact";
import { ToolContent } from "./ToolsContent";
import { fallbackCopying } from "../../utils/fallbackCopying";
import { ReasoningContent } from "./ReasoningContent";
import { MessageFooter, MessageWrapper } from "./MessageFooter";
import { ServerContentBlocks } from "./ServerContentBlocks";
import { GlobeIcon } from "@radix-ui/react-icons";
import scrollbarStyles from "../shared/scrollbar.module.css";

type ChatInputProps = {
  message: string | null;
  reasoningContent?: string | null;
  thinkingBlocks?: ThinkingBlock[] | null;
  toolCalls?: ToolCall[] | null;
  serverExecutedTools?: ToolCall[] | null;
  serverContentBlocks?: unknown[] | null;
  citations?: WebSearchCitation[] | null;
  messageId?: string;
  onBranch?: (messageId: string) => void;
  onDelete?: (messageId: string) => void;
  contextFilesByToolId?: Record<string, ChatContextFile[]>;
  diffsByToolId?: Record<string, DiffChunk[]>;
  usage?: Usage | null;
  isStreaming?: boolean;
};

const _AssistantInput: React.FC<ChatInputProps> = ({
  message,
  reasoningContent,
  thinkingBlocks,
  toolCalls,
  serverExecutedTools,
  serverContentBlocks,
  citations,
  messageId,
  onBranch,
  onDelete,
  contextFilesByToolId,
  diffsByToolId,
  usage,
  isStreaming = false,
}) => {
  // Get unique server-executed tool names for display
  const serverToolNames = useMemo(() => {
    if (!serverExecutedTools || serverExecutedTools.length === 0) return [];
    const names = serverExecutedTools
      .map((tool) => tool.function.name)
      .filter((name): name is string => !!name);
    return [...new Set(names)];
  }, [serverExecutedTools]);

  const handleCopy = useCallback((text: string) => {
    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
    if (window.navigator?.clipboard?.writeText) {
      void window.navigator.clipboard
        .writeText(text)
        .catch(() => {
          // eslint-disable-next-line no-console
          console.log("failed to copy to clipboard");
          fallbackCopying(text);
        })
        .then(() => undefined);
    } else {
      fallbackCopying(text);
    }
  }, []);

  const combinedReasoning = useMemo(() => {
    if (reasoningContent) {
      return reasoningContent;
    }
    if (thinkingBlocks && thinkingBlocks.length > 0) {
      const thinkingText = thinkingBlocks
        .filter((block) => block.thinking)
        .map((block) => block.thinking)
        .join("\n\n");
      if (thinkingText) {
        return thinkingText;
      }
    }
    return null;
  }, [reasoningContent, thinkingBlocks]);

  const handleCopyMessage = useCallback(() => {
    if (message) {
      handleCopy(message);
    }
  }, [message, handleCopy]);

  return (
    <MessageWrapper>
      {combinedReasoning && (
        <Box mb={!message ? "3" : undefined}>
          <ReasoningContent
            reasoningContent={combinedReasoning}
            onCopyClick={handleCopy}
            isStreaming={isStreaming}
            hasMessageContent={!!message}
            stateKey={messageId ? `re:${messageId}` : undefined}
          />
        </Box>
      )}

      {!!serverContentBlocks?.length && (
        <Box mb={!message && !combinedReasoning ? "3" : undefined}>
          <ServerContentBlocks blocks={serverContentBlocks} />
        </Box>
      )}
      {message && (
        <Box py="4">
          <Markdown
            canHaveInteractiveElements={true}
            onCopyClick={handleCopy}
            isStreaming={isStreaming}
          >
            {message}
          </Markdown>
        </Box>
      )}
      {/* Server-executed tools indicator with citations */}
      {(serverToolNames.length > 0 || (citations && citations.length > 0)) && (
        <Card my="3" style={{ backgroundColor: "var(--gray-a2)" }}>
          <Flex direction="column" gap="2" p="2">
            {serverToolNames.length > 0 && (
              <Flex gap="2" align="center">
                <Text size="2" color="gray">
                  <GlobeIcon />
                </Text>
                <Text size="2" color="gray">
                  {serverToolNames.join(", ")}
                </Text>
              </Flex>
            )}
            {citations && citations.length > 0 && (
              <Flex
                direction="column"
                gap="1"
                className={scrollbarStyles.scrollbarThin}
                style={{ maxHeight: "150px", overflowY: "auto" }}
              >
                <Text size="1" weight="medium" color="gray">
                  Sources:
                </Text>
                {citations
                  .filter(
                    (citation, idx, arr) =>
                      arr.findIndex((c) => c.url === citation.url) === idx,
                  )
                  .filter((citation) => {
                    try {
                      const url = new URL(citation.url);
                      return (
                        url.protocol === "http:" || url.protocol === "https:"
                      );
                    } catch {
                      return false;
                    }
                  })
                  .map((citation, idx) => (
                    <Link
                      key={idx}
                      href={citation.url}
                      target="_blank"
                      rel="noopener noreferrer"
                      size="1"
                    >
                      {citation.title}
                    </Link>
                  ))}
              </Flex>
            )}
          </Flex>
        </Card>
      )}

      {serverExecutedTools && serverExecutedTools.length > 0 && (
        <ToolContent
          toolCalls={serverExecutedTools}
          contextFilesByToolId={contextFilesByToolId}
          diffsByToolId={diffsByToolId}
        />
      )}

      {toolCalls && (
        <ToolContent
          toolCalls={toolCalls}
          contextFilesByToolId={contextFilesByToolId}
          diffsByToolId={diffsByToolId}
        />
      )}
      <MessageFooter
        messageId={messageId}
        onCopy={message ? handleCopyMessage : undefined}
        onBranch={onBranch}
        onDelete={onDelete}
        usage={usage}
      />
    </MessageWrapper>
  );
};

export const AssistantInput = React.memo(_AssistantInput);
