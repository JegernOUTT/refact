import { useMemo } from "react";
import { useAppSelector } from "../../hooks";
import {
  selectMessages,
  selectEffectiveMaxContextTokens,
} from "../../features/Chat";
import {
  isAssistantMessage,
  isUserMessage,
  isSystemMessage,
  isChatContextFileMessage,
  isToolMessage,
  isDiffMessage,
  ChatMessage,
} from "../../services/refact/types";
import type {
  TokenMap,
  TokenMapSegment,
  TokenMapItem,
  Usage,
} from "../../services/refact/chat";

function getTotalInputTokens(usage: Usage | null | undefined): number {
  if (!usage) return 0;
  return (
    usage.prompt_tokens +
    (usage.cache_creation_input_tokens ?? 0) +
    (usage.cache_read_input_tokens ?? 0)
  );
}

const PROJECT_CONTEXT_MARKER = "project_context";
const MEMORIES_CONTEXT_MARKER = "memories_context";
const TASK_MEMORIES_CONTEXT_MARKER = "task_memories_context";

type Category =
  | "system"
  | "project_context"
  | "memories"
  | "context_files"
  | "user_messages"
  | "assistant_messages"
  | "tool_results";

function getMessageTextLength(message: ChatMessage): number {
  const content = message.content;
  if (typeof content === "string") {
    return content.length;
  }
  if (Array.isArray(content)) {
    return content.reduce((acc: number, item: unknown) => {
      if (typeof item === "string") return acc + item.length;
      if (item && typeof item === "object") {
        if ("text" in item) {
          return acc + String((item as { text?: string }).text ?? "").length;
        }
        if ("file_content" in item) {
          return (
            acc +
            String((item as { file_content?: string }).file_content ?? "")
              .length
          );
        }
        if (
          "type" in item &&
          (item as { type?: string }).type === "image_url"
        ) {
          return acc;
        }
      }
      return acc + 100;
    }, 0);
  }
  return JSON.stringify(content).length;
}

function getMessageCategory(msg: ChatMessage): Category {
  if (isSystemMessage(msg)) {
    return "system";
  }
  if (isChatContextFileMessage(msg)) {
    const toolCallId = msg.tool_call_id;
    if (toolCallId === PROJECT_CONTEXT_MARKER) {
      return "project_context";
    }
    if (
      toolCallId === MEMORIES_CONTEXT_MARKER ||
      toolCallId === TASK_MEMORIES_CONTEXT_MARKER
    ) {
      return "memories";
    }
    return "context_files";
  }
  if (isUserMessage(msg)) {
    return "user_messages";
  }
  if (isAssistantMessage(msg)) {
    return "assistant_messages";
  }
  if (isToolMessage(msg) || isDiffMessage(msg)) {
    return "tool_results";
  }
  return "system";
}

function getAssistantMessageLength(msg: ChatMessage): number {
  let len = getMessageTextLength(msg);
  if (isAssistantMessage(msg) && msg.tool_calls) {
    len += JSON.stringify(msg.tool_calls).length;
  }
  return len;
}

type CategoryTokens = Record<Category, number>;

function createEmptyCategoryTokens(): CategoryTokens {
  return {
    system: 0,
    project_context: 0,
    memories: 0,
    context_files: 0,
    user_messages: 0,
    assistant_messages: 0,
    tool_results: 0,
  };
}

export function useTokenMap(enabled = true): TokenMap | null {
  const messages = useAppSelector(selectMessages);
  const maxContextTokens = useAppSelector(selectEffectiveMaxContextTokens) ?? 0;

  return useMemo(() => {
    if (!enabled) return null;
    if (messages.length === 0) return null;

    const assistantIndices: number[] = [];
    for (let i = 0; i < messages.length; i++) {
      const msg = messages[i];
      if (isAssistantMessage(msg) && getTotalInputTokens(msg.usage) > 0) {
        assistantIndices.push(i);
      }
    }

    if (assistantIndices.length === 0) return null;

    const categoryTokens = createEmptyCategoryTokens();
    const contextFileItems: {
      label: string;
      tokens: number;
      category: string;
    }[] = [];

    let prevPromptTokens = 0;
    let prevEndIndex = -1;

    for (const assistantIndex of assistantIndices) {
      const assistantMsg = messages[assistantIndex];
      const currentPromptTokens = isAssistantMessage(assistantMsg)
        ? getTotalInputTokens(assistantMsg.usage)
        : 0;
      if (currentPromptTokens === 0) continue;
      const deltaTokens = currentPromptTokens - prevPromptTokens;

      const segmentMessages: ChatMessage[] = [];
      for (let i = prevEndIndex + 1; i <= assistantIndex; i++) {
        segmentMessages.push(messages[i]);
      }

      const segmentLengths = createEmptyCategoryTokens();
      const segmentContextFiles: {
        label: string;
        length: number;
        category: string;
      }[] = [];

      for (const msg of segmentMessages) {
        const category = getMessageCategory(msg);
        const len =
          category === "assistant_messages"
            ? getAssistantMessageLength(msg)
            : getMessageTextLength(msg);

        segmentLengths[category] += len;

        if (isChatContextFileMessage(msg)) {
          for (const file of msg.content) {
            segmentContextFiles.push({
              label: file.file_name,
              length: file.file_content.length,
              category,
            });
          }
        }
      }

      const totalSegmentLength = Object.values(segmentLengths).reduce(
        (a, b) => a + b,
        0,
      );

      if (totalSegmentLength > 0 && deltaTokens > 0) {
        const scale = deltaTokens / totalSegmentLength;

        for (const cat of Object.keys(segmentLengths) as Category[]) {
          categoryTokens[cat] += Math.round(segmentLengths[cat] * scale);
        }

        for (const item of segmentContextFiles) {
          contextFileItems.push({
            label: item.label,
            tokens: Math.round(item.length * scale),
            category: item.category,
          });
        }
      }

      prevPromptTokens = currentPromptTokens;
      prevEndIndex = assistantIndex;
    }

    const lastAssistantIndex = assistantIndices[assistantIndices.length - 1];
    const lastAssistantMsg = messages[lastAssistantIndex];
    const totalPromptTokens = isAssistantMessage(lastAssistantMsg)
      ? getTotalInputTokens(lastAssistantMsg.usage)
      : 0;
    if (totalPromptTokens === 0) return null;

    const totalUsedTokens = Object.values(categoryTokens).reduce(
      (a, b) => a + b,
      0,
    );
    const freeTokens = Math.max(0, maxContextTokens - totalUsedTokens);

    const calcPercentage = (tokens: number) =>
      maxContextTokens > 0 ? (tokens / maxContextTokens) * 100 : 0;

    const segments: TokenMapSegment[] = [];

    const categoryConfig: { key: Category; label: string }[] = [
      { key: "system", label: "System prompt" },
      { key: "project_context", label: "Project context" },
      { key: "memories", label: "Memories" },
      { key: "context_files", label: "Context files" },
      { key: "user_messages", label: "User messages" },
      { key: "assistant_messages", label: "Assistant messages" },
      { key: "tool_results", label: "Tool results" },
    ];

    for (const { key, label } of categoryConfig) {
      if (categoryTokens[key] > 0) {
        segments.push({
          label,
          category: key,
          tokens: categoryTokens[key],
          percentage: calcPercentage(categoryTokens[key]),
        });
      }
    }

    if (freeTokens > 0) {
      segments.push({
        label: "Free space",
        category: "free",
        tokens: freeTokens,
        percentage: calcPercentage(freeTokens),
      });
    }

    const top_items: TokenMapItem[] = contextFileItems
      .sort((a, b) => b.tokens - a.tokens)
      .slice(0, 5)
      .map((item) => ({
        category: item.category,
        label: item.label,
        tokens: item.tokens,
      }));

    return {
      total_prompt_tokens: totalPromptTokens,
      max_context_tokens: maxContextTokens,
      estimated: false,
      segments,
      top_items,
    };
  }, [enabled, messages, maxContextTokens]);
}
