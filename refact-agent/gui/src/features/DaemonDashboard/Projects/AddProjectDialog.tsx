import { useState } from "react";

import {
  Button,
  Dialog,
  FieldError,
  FieldStack,
  FieldText,
} from "../../../components/ui";
import {
  useOpenProjectMutation,
  type DaemonProjectOpenResponse,
} from "../../../services/refact/daemon";
import { FolderBrowser } from "./FolderBrowser";
import styles from "./Projects.module.css";

type AddProjectDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onOpening: (root: string) => void;
  onProjectOpened: (response: DaemonProjectOpenResponse) => void;
  onFailed: () => void;
};

function openError(error: unknown): string | null {
  if (!error || typeof error !== "object") return null;
  if ("data" in error) {
    const data = error.data;
    if (typeof data === "string") return data;
    if (data && typeof data === "object") {
      if ("error" in data && typeof data.error === "string") return data.error;
      if ("detail" in data && typeof data.detail === "string")
        return data.detail;
    }
  }
  return "Unable to add this project.";
}

export function AddProjectDialog({
  open,
  onOpenChange,
  onOpening,
  onProjectOpened,
  onFailed,
}: AddProjectDialogProps) {
  const [path, setPath] = useState("");
  const [openProject, { error, isLoading, reset }] = useOpenProjectMutation();

  function handleOpenChange(nextOpen: boolean) {
    if (!nextOpen && !isLoading) {
      setPath("");
      reset();
    }
    if (!isLoading || nextOpen) onOpenChange(nextOpen);
  }

  async function handleAdd() {
    const root = path.trim();
    if (!root) return;
    onOpening(root);
    onOpenChange(false);
    try {
      const response = await openProject({ root }).unwrap();
      onProjectOpened(response);
      setPath("");
      onOpenChange(false);
    } catch {
      onFailed();
      onOpenChange(true);
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <Dialog.Content maxWidth="calc(var(--rf-space-6) * 18)">
        <Dialog.Title>Add project</Dialog.Title>
        <Dialog.Description>
          Enter a folder path or browse folders available to the daemon.
        </Dialog.Description>

        <div className={styles.dialogBody}>
          <FieldStack
            label="Project path"
            control={
              <FieldText
                aria-label="Project path"
                disabled={isLoading}
                onChange={setPath}
                placeholder="/path/to/project"
                value={path}
              />
            }
          />
          {open ? <FolderBrowser onSelect={setPath} /> : null}
          {error ? <FieldError>{openError(error)}</FieldError> : null}
        </div>

        <div className={styles.dialogActions}>
          <Button
            disabled={isLoading}
            onClick={() => handleOpenChange(false)}
            variant="soft"
          >
            Cancel
          </Button>
          <Button
            disabled={isLoading || !path.trim()}
            loading={isLoading}
            onClick={() => void handleAdd()}
            variant="primary"
          >
            Add project
          </Button>
        </div>
      </Dialog.Content>
    </Dialog>
  );
}
