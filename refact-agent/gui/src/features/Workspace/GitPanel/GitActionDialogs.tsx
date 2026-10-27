import { useState } from "react";

import { Checkbox } from "../../../components/Checkbox";
import { Button, Dialog } from "../../../components/ui";
import type { GitSkippedPath } from "../../../services/refact/gitRead";
import type { ChangesTreeEntry } from "./changesTreeModel";
import styles from "./GitDock.module.css";

const MAX_LISTED_FILES = 8;

function FileList({ entries }: { entries: ChangesTreeEntry[] }) {
  const shown = entries.slice(0, MAX_LISTED_FILES);
  const hidden = entries.length - shown.length;
  return (
    <ul className={styles.dialogFileList} aria-label="Affected files">
      {shown.map((entry) => (
        <li key={entry.id}>{entry.path}</li>
      ))}
      {hidden > 0 ? <li className={styles.mutedText}>+{hidden} more</li> : null}
    </ul>
  );
}

export function SkippedPaths({ skipped }: { skipped: GitSkippedPath[] }) {
  if (skipped.length === 0) return null;
  return (
    <ul className={styles.skippedList} aria-label="Skipped paths">
      {skipped.map((item) => (
        <li key={item.path}>
          <code>{item.path}</code> — {item.reason}
        </li>
      ))}
    </ul>
  );
}

type RollbackDialogProps = {
  entries: ChangesTreeEntry[];
  busy: boolean;
  error: string | null;
  onCancel: () => void;
  onConfirm: (deleteAddedFiles: boolean) => void;
};

export function RollbackDialog({
  entries,
  busy,
  error,
  onCancel,
  onConfirm,
}: RollbackDialogProps) {
  const [deleteAdded, setDeleteAdded] = useState(false);
  const hasAdded = entries.some((entry) => entry.status === "ADDED");
  const hasRenamed = entries.some(
    (entry) => entry.status === "RENAMED" && entry.oldPath !== null,
  );
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onCancel();
      }}
    >
      <Dialog.Content maxWidth="calc(var(--rf-space-6) * 14)">
        <Dialog.Title>Rollback Changes</Dialog.Title>
        <Dialog.Description>
          Restore {entries.length} {entries.length === 1 ? "file" : "files"} to
          the HEAD revision. Local modifications will be lost.
        </Dialog.Description>
        <FileList entries={entries} />
        {hasRenamed ? (
          <p className={styles.mutedText} role="note">
            Renamed files will be moved back to their original paths.
          </p>
        ) : null}
        {hasAdded ? (
          <Checkbox
            checked={deleteAdded}
            onCheckedChange={(checked) => setDeleteAdded(checked === true)}
          >
            Delete local copies of added files
          </Checkbox>
        ) : null}
        {error ? (
          <p className={styles.errorText} role="alert">
            {error}
          </p>
        ) : null}
        <Dialog.Footer>
          <Button disabled={busy} onClick={onCancel} variant="soft">
            Cancel
          </Button>
          <Button
            loading={busy}
            onClick={() => onConfirm(hasAdded && deleteAdded)}
            variant="danger"
          >
            Rollback
          </Button>
        </Dialog.Footer>
      </Dialog.Content>
    </Dialog>
  );
}

type DeleteDialogProps = {
  entries: ChangesTreeEntry[];
  busy: boolean;
  error: string | null;
  onCancel: () => void;
  onConfirm: () => void;
};

export function DeleteUnversionedDialog({
  entries,
  busy,
  error,
  onCancel,
  onConfirm,
}: DeleteDialogProps) {
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onCancel();
      }}
    >
      <Dialog.Content maxWidth="calc(var(--rf-space-6) * 14)">
        <Dialog.Title>Delete Unversioned Files</Dialog.Title>
        <Dialog.Description>
          Permanently delete {entries.length}{" "}
          {entries.length === 1 ? "file" : "files"} from disk.
        </Dialog.Description>
        <FileList entries={entries} />
        {error ? (
          <p className={styles.errorText} role="alert">
            {error}
          </p>
        ) : null}
        <Dialog.Footer>
          <Button disabled={busy} onClick={onCancel} variant="soft">
            Cancel
          </Button>
          <Button loading={busy} onClick={onConfirm} variant="danger">
            Delete
          </Button>
        </Dialog.Footer>
      </Dialog.Content>
    </Dialog>
  );
}
