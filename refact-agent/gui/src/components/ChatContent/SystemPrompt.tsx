import React from "react";
import { BookOpen } from "lucide-react";
import { Markdown } from "../Markdown";
import { Icon } from "../ui";
import { TranscriptClamp, TranscriptItem } from "./TranscriptItem";
import { formatTextSize } from "./ContextFiles.utils";

export const SystemPrompt: React.FC<{
  content: string;
}> = ({ content }) => {
  if (!content.trim()) return null;

  return (
    <TranscriptItem
      data-testid="system-prompt"
      icon={<Icon icon={BookOpen} size="sm" />}
      verb="System prompt"
      meta={formatTextSize(content)}
      sectionLabels={{ output: "" }}
      output={
        <TranscriptClamp>
          <Markdown>{content}</Markdown>
        </TranscriptClamp>
      }
    />
  );
};
