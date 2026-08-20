import React from "react";
import { Text } from "@radix-ui/themes";
import classNames from "classnames";
import { BookOpen } from "lucide-react";

import { Button, Icon } from "../ui";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { push } from "../../features/Pages/pagesSlice";
import { useSkillsStatus } from "../../hooks/useSkillsStatus";
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

  const noun = skillsAvailable === 1 ? "skill" : "skills";
  const note = skillsAvailable > 0 ? `${skillsAvailable} available` : null;
  const summary =
    activeSkill !== null
      ? `${activeSkill} is active, ${skillsAvailable} ${noun} available`
      : `${skillsAvailable} ${noun} available`;

  return (
    <div
      className={classNames(
        styles.indicator,
        activeSkill !== null && styles.indicatorActive,
      )}
      data-testid="chat-skills-indicator"
      title={summary}
    >
      <Icon
        className={styles.icon}
        icon={BookOpen}
        size="sm"
        tone={activeSkill !== null ? "accent" : "faint"}
      />
      <Text
        className={classNames(
          styles.label,
          activeSkill !== null && styles.labelMono,
        )}
        as="span"
        size="1"
      >
        {activeSkill ?? "Skills"}
      </Text>
      {note !== null && (
        <Text className={styles.note} as="span" size="1">
          {note}
        </Text>
      )}
      <span className={styles.srOnly}>{summary}</span>
      <Button
        className={styles.action}
        size="sm"
        variant="ghost"
        aria-label="Manage skills"
        onClick={() => dispatch(push({ name: "extensions", tab: "skills" }))}
      >
        Manage
      </Button>
    </div>
  );
};
