import { useId, useMemo, useState, type FormEvent } from "react";

import { Checkbox } from "../../../../components/Checkbox";
import {
  Button,
  Combobox,
  Dialog,
  Field,
  FieldText,
} from "../../../../components/ui";
import type { GitErrorView } from "../gitErrors";
import { branchNameError } from "./branchName";
import { GitErrorDetails } from "./GitFeedback";
import styles from "./Branches.module.css";

const DIALOG_WIDTH = "calc(var(--rf-space-6) * 14)";

function DialogError({ error }: { error: GitErrorView | null }) {
  if (!error) return null;
  return (
    <div className={styles.dialogError} role="alert">
      <GitErrorDetails error={error} />
    </div>
  );
}

type BaseDialogProps = {
  busy: boolean;
  error: GitErrorView | null;
  onCancel: () => void;
};

function useCloseGuard(busy: boolean, onCancel: () => void) {
  return (open: boolean) => {
    if (!open && !busy) onCancel();
  };
}

export type NewBranchDialogProps = BaseDialogProps & {
  startLabel: string;
  defaultName: string;
  existing: readonly string[];
  onConfirm: (name: string, checkout: boolean) => void;
};

export function NewBranchDialog({
  startLabel,
  defaultName,
  existing,
  busy,
  error,
  onCancel,
  onConfirm,
}: NewBranchDialogProps) {
  const inputId = useId();
  const [name, setName] = useState(defaultName);
  const [checkout, setCheckout] = useState(true);
  const [touched, setTouched] = useState(defaultName.length > 0);
  const trimmed = name.trim();
  const validation = branchNameError(trimmed, existing);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    setTouched(true);
    if (!validation && !busy) onConfirm(trimmed, checkout);
  };
  return (
    <Dialog open onOpenChange={useCloseGuard(busy, onCancel)}>
      <Dialog.Content maxWidth={DIALOG_WIDTH}>
        <Dialog.Title>Create New Branch</Dialog.Title>
        <Dialog.Description>From {startLabel}</Dialog.Description>
        <form
          className={styles.dialogForm}
          id={`${inputId}-form`}
          onSubmit={submit}
        >
          <Field
            error={touched && trimmed.length > 0 ? validation : null}
            htmlFor={inputId}
            label="New branch name"
          >
            <FieldText
              autoFocus
              id={inputId}
              onChange={(value) => {
                setName(value);
                setTouched(true);
              }}
              value={name}
            />
          </Field>
          <Checkbox
            checked={checkout}
            onCheckedChange={(checked) => setCheckout(checked === true)}
          >
            Checkout branch
          </Checkbox>
        </form>
        <DialogError error={error} />
        <Dialog.Footer>
          <Button disabled={busy} onClick={onCancel} variant="soft">
            Cancel
          </Button>
          <Button
            disabled={validation !== null}
            form={`${inputId}-form`}
            loading={busy}
            type="submit"
            variant="primary"
          >
            Create
          </Button>
        </Dialog.Footer>
      </Dialog.Content>
    </Dialog>
  );
}

export type RenameBranchDialogProps = BaseDialogProps & {
  name: string;
  existing: readonly string[];
  onConfirm: (newName: string) => void;
};

export function RenameBranchDialog({
  name: oldName,
  existing,
  busy,
  error,
  onCancel,
  onConfirm,
}: RenameBranchDialogProps) {
  const inputId = useId();
  const [name, setName] = useState(oldName);
  const trimmed = name.trim();
  const unchanged = trimmed === oldName;
  const validation = unchanged
    ? null
    : branchNameError(
        trimmed,
        existing.filter((item) => item !== oldName),
      );
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!validation && !unchanged && !busy) onConfirm(trimmed);
  };
  return (
    <Dialog open onOpenChange={useCloseGuard(busy, onCancel)}>
      <Dialog.Content maxWidth={DIALOG_WIDTH}>
        <Dialog.Title>Rename Branch</Dialog.Title>
        <Dialog.Description>
          Rename branch &apos;{oldName}&apos;
        </Dialog.Description>
        <form
          className={styles.dialogForm}
          id={`${inputId}-form`}
          onSubmit={submit}
        >
          <Field error={validation} htmlFor={inputId} label="New branch name">
            <FieldText autoFocus id={inputId} onChange={setName} value={name} />
          </Field>
        </form>
        <DialogError error={error} />
        <Dialog.Footer>
          <Button disabled={busy} onClick={onCancel} variant="soft">
            Cancel
          </Button>
          <Button
            disabled={validation !== null || unchanged}
            form={`${inputId}-form`}
            loading={busy}
            type="submit"
            variant="primary"
          >
            Rename
          </Button>
        </Dialog.Footer>
      </Dialog.Content>
    </Dialog>
  );
}

export type DeleteBranchDialogProps = BaseDialogProps & {
  name: string;
  notMerged: boolean;
  onConfirm: (force: boolean) => void;
};

export function DeleteBranchDialog({
  name,
  notMerged,
  busy,
  error,
  onCancel,
  onConfirm,
}: DeleteBranchDialogProps) {
  return (
    <Dialog open onOpenChange={useCloseGuard(busy, onCancel)}>
      <Dialog.Content maxWidth={DIALOG_WIDTH}>
        <Dialog.Title>Delete Branch</Dialog.Title>
        <Dialog.Description>
          Delete the local branch &apos;{name}&apos;?
        </Dialog.Description>
        {notMerged ? (
          <p className={styles.warningText} role="alert">
            &apos;{name}&apos; is not fully merged. Its unmerged commits will
            only stay reachable through the reflog.
          </p>
        ) : (
          <DialogError error={error} />
        )}
        <Dialog.Footer>
          <Button disabled={busy} onClick={onCancel} variant="soft">
            Cancel
          </Button>
          <Button
            loading={busy}
            onClick={() => onConfirm(notMerged)}
            variant="danger"
          >
            {notMerged ? "Force Delete" : "Delete"}
          </Button>
        </Dialog.Footer>
      </Dialog.Content>
    </Dialog>
  );
}

export type CheckoutRevisionDialogProps = BaseDialogProps & {
  tags: readonly string[];
  onConfirm: (revision: string, isTag: boolean) => void;
};

export function CheckoutRevisionDialog({
  tags,
  busy,
  error,
  onCancel,
  onConfirm,
}: CheckoutRevisionDialogProps) {
  const formId = useId();
  const [revision, setRevision] = useState("");
  const items = useMemo(() => tags.map((tag) => ({ value: tag })), [tags]);
  const trimmed = revision.trim();
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (trimmed && !busy) onConfirm(trimmed, tags.includes(trimmed));
  };
  return (
    <Dialog open onOpenChange={useCloseGuard(busy, onCancel)}>
      <Dialog.Content maxWidth={DIALOG_WIDTH}>
        <Dialog.Title>Checkout Tag or Revision</Dialog.Title>
        <Dialog.Description>
          Enter a tag, commit hash or any revision expression. HEAD will be
          detached.
        </Dialog.Description>
        <form className={styles.dialogForm} id={formId} onSubmit={submit}>
          <Combobox
            aria-label="Tag or revision"
            autoFocus
            emptyLabel="No matching tags"
            items={items}
            onSelect={(item) => setRevision(item.value)}
            onValueChange={setRevision}
            placeholder="Tag or revision"
            value={revision}
          />
        </form>
        <DialogError error={error} />
        <Dialog.Footer>
          <Button disabled={busy} onClick={onCancel} variant="soft">
            Cancel
          </Button>
          <Button
            disabled={!trimmed}
            form={formId}
            loading={busy}
            type="submit"
            variant="primary"
          >
            Checkout
          </Button>
        </Dialog.Footer>
      </Dialog.Content>
    </Dialog>
  );
}
