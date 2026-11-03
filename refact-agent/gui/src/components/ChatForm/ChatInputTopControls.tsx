import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  CircleHelp,
  Info,
  Lock,
  Pencil,
  Plus,
  TriangleAlert,
  Unlock,
} from "lucide-react";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useAppSelector } from "../../hooks/useAppSelector";
import {
  selectAutoApproveEditingToolsById,
  selectAutoApproveDangerousCommandsById,
  selectIncludeProjectInfoById,
  useThreadId,
} from "../../features/Chat/Thread";
import {
  setAutoApproveEditingTools,
  setAutoApproveDangerousCommands,
} from "../../features/Chat/Thread/actions";
import { ProjectInformationDialog } from "./ProjectInformationDialog";
import { selectHost } from "../../features/Config/configSlice";
import { Checkbox } from "../Checkbox";
import { ComposerTrigger, KitTooltip } from "../ComposerKit";
import type { Checkbox as CheckboxType } from "./useCheckBoxes";
import type { useAttachedFiles } from "./useCheckBoxes";
import styles from "./ChatInputTopControls.module.css";

export type ChatInputTopControlsProps = {
  checkboxes: Record<string, CheckboxType>;
  onCheckedChange: (name: string, checked: boolean | string) => void;
  attachedFiles: ReturnType<typeof useAttachedFiles>;
  disabled?: boolean;
  onDialogOpenChange?: (open: boolean) => void;
  renderControls?: (
    controls: Record<string, React.ReactNode>,
    dangerous: boolean,
  ) => React.ReactNode;
};

export const ChatInputTopControls: React.FC<ChatInputTopControlsProps> = ({
  checkboxes,
  onCheckedChange,
  attachedFiles,
  disabled,
  onDialogOpenChange,
  renderControls,
}) => {
  const isDisabled = disabled ?? false;
  const dispatch = useAppDispatch();
  const host = useAppSelector(selectHost);
  const chatId = useThreadId();
  const autoApproveEditing = useAppSelector((state) =>
    selectAutoApproveEditingToolsById(state, chatId),
  );
  const autoApproveDangerous = useAppSelector((state) =>
    selectAutoApproveDangerousCommandsById(state, chatId),
  );
  const includeProjectInfo = useAppSelector((state) =>
    selectIncludeProjectInfoById(state, chatId),
  );
  const [dialogOpen, setDialogOpen] = useState(false);
  const projectInfoTriggerRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!dialogOpen) return;
    const trigger = projectInfoTriggerRef.current;
    onDialogOpenChange?.(true);
    return () => {
      trigger?.focus({ preventScroll: true });
      onDialogOpenChange?.(false);
    };
  }, [dialogOpen, onDialogOpenChange]);

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

  const controls = {
    projectInfo: (
      <KitTooltip
        label="Project info"
        state={includeProjectInfo ? "On" : "Off"}
        stateTone={includeProjectInfo ? "accent" : undefined}
      >
        <ComposerTrigger
          ref={projectInfoTriggerRef}
          iconOnly
          icon={Info}
          aria-label="Configure project information"
          aria-haspopup="dialog"
          aria-expanded={dialogOpen}
          disabled={isDisabled}
          className={includeProjectInfo ? styles.accent : undefined}
          onClick={() => setDialogOpen(true)}
        />
      </KitTooltip>
    ),
    editApproval: (
      <KitTooltip
        label="Auto-approve edits"
        state={autoApproveEditing ? "On" : "Off"}
        stateTone={autoApproveEditing ? "accent" : undefined}
      >
        <ComposerTrigger
          iconOnly
          icon={Pencil}
          aria-label="Auto-approve file editing tools"
          aria-pressed={autoApproveEditing}
          disabled={isDisabled || !chatId}
          onClick={() => handleEditingChange(!autoApproveEditing)}
        />
      </KitTooltip>
    ),
    dangerousApproval: (
      <KitTooltip
        label="Auto-approve dangerous commands"
        state={autoApproveDangerous ? "On" : "Off"}
        stateTone={autoApproveDangerous ? "danger" : undefined}
      >
        <ComposerTrigger
          iconOnly
          tone="danger"
          icon={TriangleAlert}
          aria-label="Auto-approve dangerous commands"
          aria-pressed={autoApproveDangerous}
          disabled={isDisabled || !chatId}
          onClick={() => handleDangerousChange(!autoApproveDangerous)}
        />
      </KitTooltip>
    ),
    selectedLines: showSelectedLines && (
      <span className={styles.selectedLinesGroup}>
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
        <ComposerTrigger
          iconOnly
          icon={selectedLinesCheckbox.locked ? Lock : Unlock}
          aria-label={selectedLinesCheckbox.locked ? "Locked" : "Unlocked"}
          disabled={isDisabled || selectedLinesCheckbox.disabled}
          onClick={() =>
            onCheckedChange(
              selectedLinesCheckbox.name,
              !selectedLinesCheckbox.checked,
            )
          }
        />
        {selectedLinesCheckbox.info && (
          <KitTooltip
            label="Selected lines"
            hint={selectedLinesCheckbox.info.text}
          >
            <ComposerTrigger
              iconOnly
              icon={CircleHelp}
              aria-label="Selected lines information"
              disabled={isDisabled}
            />
          </KitTooltip>
        )}
      </span>
    ),
    activeFile: showAttachButton && (
      <KitTooltip label="Attach" state={attachedFiles.activeFile.name}>
        <ComposerTrigger
          iconOnly
          icon={Plus}
          aria-label={`Attach ${attachedFiles.activeFile.name}`}
          aria-pressed={attachedFiles.attached}
          disabled={isDisabled || attachedFiles.attached}
          onClick={attachedFiles.addFile}
        />
      </KitTooltip>
    ),
  };
  return (
    <>
      {renderControls ? (
        renderControls(controls, autoApproveDangerous)
      ) : (
        <div className={styles.controlsGroup}>{Object.values(controls)}</div>
      )}
      <ProjectInformationDialog
        chatId={chatId}
        open={dialogOpen}
        onOpenChange={setDialogOpen}
      />
    </>
  );
};
