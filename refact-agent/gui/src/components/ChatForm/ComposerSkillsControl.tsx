import React, { useCallback, useState } from "react";
import classNames from "classnames";
import { BookOpen } from "lucide-react";

import { Popover } from "../ui";
import {
  ComposerTrigger,
  KitChevron,
  KitDivider,
  KitHeader,
  KitPanel,
  KitRow,
  KitStaticRow,
  kit,
} from "../ComposerKit";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useSkillsStatus } from "../../hooks/useSkillsStatus";
import { push } from "../../features/Pages/pagesSlice";
import styles from "./ComposerSkillsControl.module.css";

export type ComposerSkillsControlProps = {
  chatId: string;
  disabled?: boolean;
  onOpenChange?: (open: boolean) => void;
};

export const ComposerSkillsControl: React.FC<ComposerSkillsControlProps> = ({
  chatId,
  disabled = false,
  onOpenChange,
}) => {
  const dispatch = useAppDispatch();
  const { skillsAvailable, activeSkill } = useSkillsStatus(chatId);
  const [open, setOpen] = useState(false);

  const handleOpenChange = useCallback(
    (next: boolean) => {
      setOpen(next);
      onOpenChange?.(next);
    },
    [onOpenChange],
  );

  const noun = skillsAvailable === 1 ? "skill" : "skills";
  const available =
    skillsAvailable > 0
      ? `${skillsAvailable} ${noun} available`
      : "No skills installed";
  const summary =
    activeSkill !== null
      ? `${activeSkill} is active · ${available}`
      : available;

  return (
    <Popover responsive={false} open={open} onOpenChange={handleOpenChange}>
      <Popover.Trigger asChild>
        <ComposerTrigger
          icon={BookOpen}
          className={classNames(activeSkill !== null && styles.active)}
          aria-label={`Skills: ${summary}`}
          disabled={disabled}
          data-testid="composer-skills-control"
        >
          <span className={kit.label} data-shrink-at="1">
            {activeSkill ?? "Skills"}
          </span>
        </ComposerTrigger>
      </Popover.Trigger>
      <Popover.Content
        className={kit.popover}
        align="end"
        maxWidth="260px"
        side="top"
      >
        <KitPanel>
          <KitHeader title="Skills" meta={`${skillsAvailable} available`} />
          {activeSkill !== null ? (
            <KitStaticRow
              icon={<BookOpen aria-hidden="true" />}
              label={activeSkill}
              meta="active"
              strong
            />
          ) : (
            <KitStaticRow
              tone="muted"
              label={
                skillsAvailable > 0 ? "No active skill" : "No skills installed"
              }
            />
          )}
          <KitDivider />
          <KitRow
            label="Manage skills"
            trailing={<KitChevron />}
            onClick={() => {
              handleOpenChange(false);
              dispatch(push({ name: "extensions", tab: "skills" }));
            }}
          />
        </KitPanel>
      </Popover.Content>
    </Popover>
  );
};
