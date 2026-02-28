import React from "react";
import { Flex, Text } from "@radix-ui/themes";
import { useAppDispatch } from "../../hooks";
import { push } from "../../features/Pages/pagesSlice";
import { useSkillsStatus } from "../../hooks/useSkillsStatus";
import styles from "./SkillsIndicator.module.css";

export type SkillsIndicatorProps = {
  chatId: string;
};

export const SkillsIndicator: React.FC<SkillsIndicatorProps> = ({ chatId }) => {
  const dispatch = useAppDispatch();
  const { skillsEnabled, skillsAvailable, skillsIncluded } =
    useSkillsStatus(chatId);

  if (!skillsEnabled || skillsAvailable === 0) {
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

  const activeText =
    skillsIncluded.length > 0
      ? `${skillsIncluded.length} active (${skillsIncluded.join(", ")}) · `
      : "";

  return (
    <Flex
      align="center"
      className={styles.indicator}
      role="button"
      tabIndex={0}
      aria-label="Click to manage skills"
      title="Click to manage skills"
      onClick={handleClick}
      onKeyDown={handleKeyDown}
    >
      <Text size="1" color="gray">
        🧠 Skills: {activeText}
        {skillsAvailable} available
      </Text>
    </Flex>
  );
};
