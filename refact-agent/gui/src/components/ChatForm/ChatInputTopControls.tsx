import React, { useCallback, useState } from "react";
import classNames from "classnames";
import {
  CircleHelp,
  Info,
  Lock,
  Pencil,
  Plus,
  TriangleAlert,
  Unlock,
} from "lucide-react";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectAutoApproveEditingTools,
  selectAutoApproveDangerousCommands,
  selectCurrentThreadId,
  selectIncludeProjectInfo,
} from "../../features/Chat";
import {
  setAutoApproveEditingTools,
  setAutoApproveDangerousCommands,
} from "../../features/Chat/Thread/actions";
import { ProjectInformationDialog } from "./ProjectInformationDialog";
import { selectHost } from "../../features/Config/configSlice";
import { Checkbox } from "../Checkbox";
import { Tooltip } from "../ui";
import type { Checkbox as CheckboxType } from "./useCheckBoxes";
import type { useAttachedFiles } from "./useCheckBoxes";
import styles from "./ChatInputTopControls.module.css";

export type ChatInputTopControlsProps = {
  checkboxes: Record<string, CheckboxType>;
  onCheckedChange: (name: string, checked: boolean | string) => void;
  attachedFiles: ReturnType<typeof useAttachedFiles>;
  disabled?: boolean;
};

export const ChatInputTopControls: React.FC<ChatInputTopControlsProps> = ({
  checkboxes,
  onCheckedChange,
  attachedFiles,
  disabled,
}) => {
  const isDisabled = disabled ?? false;
  const dispatch = useAppDispatch();
  const host = useAppSelector(selectHost);
  const chatId = useAppSelector(selectCurrentThreadId);
  const autoApproveEditing = useAppSelector(selectAutoApproveEditingTools);
  const autoApproveDangerous = useAppSelector(
    selectAutoApproveDangerousCommands,
  );
  const includeProjectInfo = useAppSelector(selectIncludeProjectInfo);
  const [dialogOpen, setDialogOpen] = useState(false);

  const handleEditingChange = useCallback(
    (checked: boolean) => {
      if (chatId) {
        dispatch(setAutoApproveEditingTools({ chatId, value: checked }));
      }
    },
    [dispatch, chatId],
  );

  const handleDangerousChange = useCallback(
    (checked: boolean) => {
      if (chatId) {
        dispatch(setAutoApproveDangerousCommands({ chatId, value: checked }));
      }
    },
    [dispatch, chatId],
  );

  const selectedLinesCheckbox = checkboxes.selected_lines;
  const showSelectedLines = host !== "web" && !selectedLinesCheckbox.hide;
  const showAttachButton = host !== "web" && attachedFiles.activeFile.name;

  return (
    <>
      <div className={styles.controlsGroup}>
        <span className={styles.projectInfoControl}>
          <Tooltip>
            <Tooltip.Trigger asChild>
              <button
                type="button"
                onClick={() => setDialogOpen(true)}
                disabled={isDisabled}
                aria-label="Configure project information"
                className={classNames(
                  styles.iconButton,
                  includeProjectInfo && styles.active,
                )}
              >
                <Info />
              </button>
            </Tooltip.Trigger>
            <Tooltip.Content>
              Project info: {includeProjectInfo ? "ON" : "OFF"}
            </Tooltip.Content>
          </Tooltip>
        </span>

        <Tooltip>
          <Tooltip.Trigger asChild>
            <button
              type="button"
              onClick={() => handleEditingChange(!autoApproveEditing)}
              disabled={isDisabled || !chatId}
              aria-label="Auto-approve file editing tools"
              aria-pressed={autoApproveEditing}
              className={classNames(
                styles.iconButton,
                autoApproveEditing && styles.active,
              )}
            >
              <Pencil />
            </button>
          </Tooltip.Trigger>
          <Tooltip.Content>
            Auto-approve edits: {autoApproveEditing ? "ON" : "OFF"}
          </Tooltip.Content>
        </Tooltip>

        <Tooltip>
          <Tooltip.Trigger asChild>
            <button
              type="button"
              onClick={() => handleDangerousChange(!autoApproveDangerous)}
              disabled={isDisabled || !chatId}
              aria-label="Auto-approve dangerous commands"
              aria-pressed={autoApproveDangerous}
              className={classNames(
                styles.iconButton,
                autoApproveDangerous && styles.danger,
              )}
            >
              <TriangleAlert />
            </button>
          </Tooltip.Trigger>
          <Tooltip.Content>
            Auto-approve dangerous: {autoApproveDangerous ? "ON" : "OFF"}
          </Tooltip.Content>
        </Tooltip>

        {showSelectedLines && (
          <>
            <span className={styles.divider}>|</span>
            <div className={styles.selectedLinesGroup}>
              <Checkbox
                size="1"
                name={selectedLinesCheckbox.name}
                checked={selectedLinesCheckbox.checked}
                disabled={isDisabled || selectedLinesCheckbox.disabled}
                onCheckedChange={(value) =>
                  onCheckedChange(selectedLinesCheckbox.name, value)
                }
              >
                <span>{selectedLinesCheckbox.label}</span>
              </Checkbox>
              <button
                type="button"
                className={styles.lockButton}
                onClick={() =>
                  onCheckedChange(
                    selectedLinesCheckbox.name,
                    !selectedLinesCheckbox.checked,
                  )
                }
                disabled={isDisabled || selectedLinesCheckbox.disabled}
                aria-label={
                  selectedLinesCheckbox.locked ? "Locked" : "Unlocked"
                }
              >
                {selectedLinesCheckbox.locked ? <Lock /> : <Unlock />}
              </button>
              {selectedLinesCheckbox.info && (
                <Tooltip>
                  <Tooltip.Trigger asChild>
                    <button
                      type="button"
                      className={styles.helpButton}
                      disabled={isDisabled}
                      aria-label="Selected lines information"
                    >
                      <CircleHelp />
                    </button>
                  </Tooltip.Trigger>
                  <Tooltip.Content maxWidth="240px">
                    {selectedLinesCheckbox.info.text}
                  </Tooltip.Content>
                </Tooltip>
              )}
            </div>
          </>
        )}

        {showAttachButton && (
          <>
            <span className={styles.divider}>|</span>
            <Tooltip>
              <Tooltip.Trigger asChild>
                <button
                  type="button"
                  onClick={attachedFiles.addFile}
                  disabled={isDisabled || attachedFiles.attached}
                  aria-label={`Attach ${attachedFiles.activeFile.name}`}
                  className={classNames(
                    styles.iconButton,
                    attachedFiles.attached && styles.active,
                  )}
                >
                  <Plus />
                </button>
              </Tooltip.Trigger>
              <Tooltip.Content>
                Attach: {attachedFiles.activeFile.name}
              </Tooltip.Content>
            </Tooltip>
          </>
        )}
      </div>

      <ProjectInformationDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
      />
    </>
  );
};
