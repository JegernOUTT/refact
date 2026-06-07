import React, { useMemo } from "react";
import { Zap } from "lucide-react";
import { ToolCard } from "./ToolCard/ToolCard";
import { useStoredOpen } from "./useStoredOpen";
import { Markdown } from "../Markdown";
import { Icon } from "../ui";
import styles from "./SkillActivatedCard.module.css";

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
  const storeKey = `skill:${name}`;
  const [isOpen, handleToggle] = useStoredOpen(storeKey, false);

  const meta = useMemo(() => {
    const parts: string[] = [];
    if (modelOverride) parts.push(modelOverride);
    if (allowedTools.length > 0)
      parts.push(`tools: ${allowedTools.join(", ")}`);
    return parts.length > 0 ? parts.join(" · ") : undefined;
  }, [allowedTools, modelOverride]);

  return (
    <ToolCard
      icon={<Icon icon={Zap} size="sm" tone="accent" />}
      summary={
        <span className={styles.skillText}>
          Skill active: <span className={styles.skillName}>{name}</span>
        </span>
      }
      meta={meta}
      status="success"
      isOpen={isOpen}
      onToggle={handleToggle}
      className={styles.skillCard}
    >
      {body && (
        <div className={styles.body}>
          <Markdown>{body}</Markdown>
        </div>
      )}
    </ToolCard>
  );
};
