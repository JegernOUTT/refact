import React from "react";
import { Badge, Icon } from "../ui";
import { useAppDispatch } from "../../hooks";
import { push } from "../../features/Pages/pagesSlice";
import { useSkillsStatus } from "../../hooks/useSkillsStatus";
import { BookOpen } from "lucide-react";
import styles from "./SkillsIndicator.module.css";

export type SkillsIndicatorProps = {
  chatId: string;
};

export const SkillsIndicator: React.FC<SkillsIndicatorProps> = ({ chatId }) => {
  const dispatch = useAppDispatch();
  const { skillsAvailable, activeSkill } = useSkillsStatus(chatId);

  if (activeSkill === null && skillsAvailable === 0) {
    return null;
  }

  const handleClick = () => {
    dispatch(push({ name: "extensions", tab: "skills" }));
  };

  const handleKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      handleClick();
    }
  };

  return (
    <div
      className={styles.indicator}
      role="button"
      tabIndex={0}
      aria-label="Click to manage skills"
      title="Click to manage skills"
      onClick={handleClick}
      onKeyDown={handleKeyDown}
    >
      <Icon icon={BookOpen} size="sm" tone="muted" />
      {activeSkill !== null ? (
        <>
          <span className={styles.muted}>Active skill:</span>
          <Badge tone="accent">{activeSkill}</Badge>
          {skillsAvailable > 0 && (
            <span className={styles.muted}>· {skillsAvailable} available</span>
          )}
        </>
      ) : (
        <span className={styles.muted}>
          Skills: {skillsAvailable} available
        </span>
      )}
    </div>
  );
};
