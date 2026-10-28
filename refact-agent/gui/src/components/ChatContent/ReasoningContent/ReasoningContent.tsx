import React, { useEffect, useMemo, useRef, useState } from "react";
import { useStoredOpen } from "../useStoredOpen";
import { Zap } from "lucide-react";
import { Icon } from "../../ui";
import { Markdown } from "../../Markdown";
import { useStreamingMarkdown } from "../../Markdown/useStreamingMarkdown";
import {
  addBuddyCrashBreadcrumb,
  setBuddyCrashHotSlot,
} from "../../../features/Buddy/reportBuddyFrontendError";
import { TranscriptClamp, TranscriptItem } from "../TranscriptItem";

// Bold titles glued to the previous sentence ("done.**Next Step**") get a
// paragraph break so Markdown renders each title as its own block.
function fixReasoningParagraphs(text: string): string {
  return text.replace(/(\S)(\*\*[A-Z])/g, "$1\n\n$2");
}

const MARKDOWN_MARKS = /[*_`#>]+/g;
const MAX_HINT_LENGTH = 80;

function plainLine(line: string): string {
  return line.replace(MARKDOWN_MARKS, "").replace(/\s+/g, " ").trim();
}

function meaningfulLines(text: string): string[] {
  return text
    .split("\n")
    .map(plainLine)
    .filter((line) => line.length > 0);
}

function truncate(text: string): string {
  return text.length > MAX_HINT_LENGTH
    ? `${text.slice(0, MAX_HINT_LENGTH - 1)}…`
    : text;
}

function formatDuration(seconds: number): string {
  if (seconds < 60) return `${Math.max(1, Math.round(seconds))}s`;
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = Math.round(seconds % 60);
  return remainingSeconds === 0
    ? `${minutes}m`
    : `${minutes}m ${remainingSeconds}s`;
}

function stepCount(text: string): number {
  return (text.match(/(^|\n)\s*\*\*[A-Z][^*\n]*\*\*/g) ?? []).length;
}

type ReasoningContentProps = {
  reasoningContent: string;
  onCopyClick: (text: string) => void;
  isStreaming?: boolean;
  hasMessageContent?: boolean;
  stateKey?: string;
};

function useThinkingDuration(isActivelyThinking: boolean): number | null {
  const [duration, setDuration] = useState<number | null>(null);
  const startRef = useRef<number | null>(null);
  useEffect(() => {
    if (isActivelyThinking) {
      startRef.current ??= Date.now();
      return;
    }
    if (startRef.current !== null) {
      setDuration((Date.now() - startRef.current) / 1000);
      startRef.current = null;
    }
  }, [isActivelyThinking]);
  return duration;
}

export const ReasoningContent: React.FC<ReasoningContentProps> = ({
  reasoningContent,
  onCopyClick,
  isStreaming = false,
  hasMessageContent = false,
  stateKey,
}) => {
  const isActivelyThinking =
    isStreaming && !!reasoningContent && !hasMessageContent;
  const duration = useThinkingDuration(isActivelyThinking);
  const [open, , setOpen] = useStoredOpen(stateKey, false);

  const formattedContent = useMemo(
    () => fixReasoningParagraphs(reasoningContent),
    [reasoningContent],
  );
  const lines = useMemo(
    () => meaningfulLines(formattedContent),
    [formattedContent],
  );
  const deferredContent = useStreamingMarkdown(
    formattedContent,
    isStreaming && open,
  );

  useEffect(() => {
    if (isStreaming && open) {
      const text = deferredContent ?? formattedContent;
      setBuddyCrashHotSlot("reasoning", text);
      addBuddyCrashBreadcrumb("reasoning", text);
      return;
    }
    setBuddyCrashHotSlot("reasoning", null);
  }, [deferredContent, formattedContent, open, isStreaming]);

  const target = lines.length > 0 ? truncate(lines[0]) : null;
  const steps = stepCount(formattedContent);
  const meta = isActivelyThinking
    ? lines.length > 1
      ? truncate(lines[lines.length - 1])
      : null
    : duration !== null
      ? formatDuration(duration)
      : steps > 1
        ? `${steps} steps`
        : null;

  return (
    <TranscriptItem
      data-testid="reasoning-content"
      icon={<Icon icon={Zap} size="sm" />}
      verb="Thought"
      target={target}
      meta={meta}
      status={isActivelyThinking ? "running" : "done"}
      open={open}
      onOpenChange={setOpen}
      sectionLabels={{ output: "" }}
      output={
        <TranscriptClamp>
          <Markdown
            canHaveInteractiveElements={true}
            onCopyClick={onCopyClick}
            isStreaming={isStreaming}
          >
            {deferredContent ?? formattedContent}
          </Markdown>
        </TranscriptClamp>
      }
    />
  );
};
