import React from "react";
import classNames from "classnames";
import { CheckSquare, ChevronDown, Plus } from "lucide-react";

import { Icon, IconButton, Menu, Tooltip } from "../ui";
import styles from "./NewSplitButton.module.css";

export type NewSplitButtonProps = {
  onCreateNewChat: () => void;
  onCreateNewTask: () => void;
};

export const NewSplitButton: React.FC<NewSplitButtonProps> = ({
  onCreateNewChat,
  onCreateNewTask,
}) => {
  return (
    <div className={styles.group}>
      <Tooltip>
        <Tooltip.Trigger asChild>
          <IconButton
            aria-label="New Chat"
            className={classNames(styles.mainAction, "rf-pressable")}
            icon={Plus}
            onClick={onCreateNewChat}
            size="sm"
            variant="plain"
          />
        </Tooltip.Trigger>
        <Tooltip.Content side="bottom">New Chat</Tooltip.Content>
      </Tooltip>

      <span className={styles.divider} />

      <Menu>
        <Tooltip>
          <Tooltip.Trigger asChild>
            <Menu.Trigger asChild>
              <IconButton
                aria-label="More new actions"
                className={classNames(styles.menuAction, "rf-pressable")}
                icon={ChevronDown}
                size="sm"
                variant="plain"
              />
            </Menu.Trigger>
          </Tooltip.Trigger>
          <Tooltip.Content side="bottom">More new actions</Tooltip.Content>
        </Tooltip>

        <Menu.Content align="end">
          <Menu.Item onSelect={onCreateNewTask}>
            <Icon icon={CheckSquare} size="sm" /> New Task
          </Menu.Item>
        </Menu.Content>
      </Menu>
    </div>
  );
};
