import React from "react";
import { Badge, Flex, Text } from "@radix-ui/themes";
import { useAppDispatch } from "../../hooks";
import { push } from "../../features/Pages/pagesSlice";
import { useSkillsStatus } from "../../hooks/useSkillsStatus";
import { ReaderIcon } from "@radix-ui/react-icons";
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
    <Flex
      align="center"
      gap="2"
      className={styles.indicator}
      role="button"
      tabIndex={0}
      aria-label="Click to manage skills"
      title="Click to manage skills"
      onClick={handleClick}
      onKeyDown={handleKeyDown}
    >
      <Text size="1" color="gray">
        <ReaderIcon />
      </Text>
      {activeSkill !== null ? (
        <>
          <Text size="1" color="gray">
            Active skill:
          </Text>
          <Badge size="1" variant="solid">
            {activeSkill}
          </Badge>
          {skillsAvailable > 0 && (
            <Text size="1" color="gray">
              · {skillsAvailable} available
            </Text>
          )}
        </>
      ) : (
        <Text size="1" color="gray">
          Skills: {skillsAvailable} available
        </Text>
      )}
    </Flex>
  );
};
