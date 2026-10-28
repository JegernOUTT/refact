import React from "react";
import { Zap } from "lucide-react";
import { Markdown } from "../Markdown";
import { TranscriptClamp, TranscriptItem } from "./TranscriptItem";

interface SkillActivatedCardProps {
  name: string;
  body: string;
  allowedTools: string[];
  modelOverride: string | null;
}

export const SkillActivatedCard: React.FC<SkillActivatedCardProps> = ({
  name,
  body,
  allowedTools,
  modelOverride,
}) => {
  const summary = [
    ...(modelOverride ? [{ label: "Model", value: modelOverride }] : []),
    ...(allowedTools.length > 0
      ? [{ label: "Tools", value: allowedTools.join(", ") }]
      : []),
  ];
  return (
    <TranscriptItem
      data-testid="skill-activated-card"
      icon={<Zap />}
      input={
        body ? (
          <TranscriptClamp>
            <Markdown>{body}</Markdown>
          </TranscriptClamp>
        ) : null
      }
      meta="activated"
      openKey={`skill:${name}`}
      sectionLabels={{ input: "Instructions" }}
      status="done"
      summary={summary}
      target={name}
      verb="Skill"
    />
  );
};
