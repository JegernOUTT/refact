import React from "react";
import { FileText } from "lucide-react";
import { Markdown } from "./ContextFiles";
import { TranscriptClamp, TranscriptItem } from "./TranscriptItem";

export type PlainTextProps = {
  children: string;
  id?: string;
  defaultOpen?: boolean;
};

const PREVIEW_CHARS = 100;

export const PlainText: React.FC<PlainTextProps> = ({
  children,
  id,
  defaultOpen = false,
}) => {
  const text = "```text\n" + children + "\n```";
  const flat = children.replace(/\s+/gu, " ").trim();
  const preview =
    flat.length > PREVIEW_CHARS ? `${flat.slice(0, PREVIEW_CHARS)}…` : flat;

  return (
    <TranscriptItem
      data-plain-text-id={id}
      data-testid="plain-text"
      defaultOpen={defaultOpen}
      icon={<FileText aria-hidden />}
      openKey={id ? `plaintext:${id}` : undefined}
      output={
        <TranscriptClamp lineCount={children.split("\n").length}>
          <Markdown>{text}</Markdown>
        </TranscriptClamp>
      }
      sectionLabels={{ output: "Text" }}
      status="done"
      target={preview || "Plain text"}
      verb="Note"
    />
  );
};
