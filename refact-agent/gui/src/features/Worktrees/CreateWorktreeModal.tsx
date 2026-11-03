import React, {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { FieldError, FieldText } from "../../components/ui";
import {
  KitButton,
  KitDialog,
  KitDialogCancel,
  KitRow,
  kit,
} from "../../components/ComposerKit";
import { dialogNonInteractiveCloseHandlers } from "../../utils/dialogPointerClose";
import styles from "./Worktrees.module.css";

export type CreateWorktreeValues = {
  branch?: string;
  baseBranch?: string;
};

type CreateWorktreeModalProps = {
  open: boolean;
  defaultBranch: string;
  defaultBaseBranch: string;
  baseBranchOptions: string[];
  isCreating: boolean;
  error?: string | null;
  onOpenChange: (open: boolean) => void;
  onCreate: (values: CreateWorktreeValues) => Promise<void>;
};

export const CreateWorktreeModal: React.FC<CreateWorktreeModalProps> = ({
  open,
  defaultBranch,
  defaultBaseBranch,
  baseBranchOptions,
  isCreating,
  error,
  onOpenChange,
  onCreate,
}) => {
  const [branchName, setBranchName] = useState(defaultBranch);
  const [baseBranch, setBaseBranch] = useState(defaultBaseBranch);
  const [baseBranchPickerOpen, setBaseBranchPickerOpen] = useState(false);
  const [baseBranchSearchTouched, setBaseBranchSearchTouched] = useState(false);
  const branchErrorId = useId();
  const branchInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) {
      setBranchName(defaultBranch);
      setBaseBranch(defaultBaseBranch);
      setBaseBranchPickerOpen(false);
      setBaseBranchSearchTouched(false);
    }
  }, [open, defaultBranch, defaultBaseBranch]);

  const normalizedBaseOptions = useMemo(() => {
    const seen = new Set<string>();
    return baseBranchOptions
      .concat(defaultBaseBranch)
      .map((branch) => branch.trim())
      .filter((branch) => branch.length > 0)
      .filter((branch) => {
        if (seen.has(branch)) return false;
        seen.add(branch);
        return true;
      });
  }, [baseBranchOptions, defaultBaseBranch]);

  const handleCreate = useCallback(async () => {
    await onCreate({
      branch: branchName.trim() || undefined,
      baseBranch: baseBranch.trim() || undefined,
    });
  }, [baseBranch, branchName, onCreate]);

  const canCreate = !isCreating && baseBranch.trim().length > 0;
  /*
   * N-58: the create failure describes the branch name the user typed, so it
   * belongs to that field: it renders under the owning FieldText, marks the
   * input invalid and is wired with aria-describedby. FieldError supplies
   * role="alert" so assistive tech announces it.
   */
  const branchError = error ?? undefined;
  const filteredBaseOptions = useMemo(() => {
    const query = baseBranchSearchTouched
      ? baseBranch.trim().toLowerCase()
      : "";
    const options = query
      ? normalizedBaseOptions.filter((branch) =>
          branch.toLowerCase().includes(query),
        )
      : normalizedBaseOptions;
    return options.slice(0, 8);
  }, [baseBranch, baseBranchSearchTouched, normalizedBaseOptions]);

  return (
    <KitDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Create worktree"
      description="Create a new git worktree and attach it to this chat."
      width="440px"
      onOpenAutoFocus={(event) => {
        event.preventDefault();
        branchInputRef.current?.focus();
      }}
      bodyProps={dialogNonInteractiveCloseHandlers(() => onOpenChange(false))}
      footer={
        <>
          <KitDialogCancel disabled={isCreating} />
          <KitButton
            variant="primary"
            onClick={() => void handleCreate()}
            disabled={!canCreate}
            aria-busy={isCreating || undefined}
          >
            {isCreating ? "Creating..." : "Create"}
          </KitButton>
        </>
      }
    >
      <div className={styles.dialogContentWrapper}>
        <div className={styles.field}>
          <label className={styles.fieldLabelGroup}>
            <span className={kit.section}>Branch name</span>
            <span className={styles.fieldControl}>
              <FieldText
                ref={branchInputRef}
                value={branchName}
                placeholder={defaultBranch}
                onChange={setBranchName}
                disabled={isCreating}
                aria-invalid={branchError ? true : undefined}
                aria-describedby={branchError ? branchErrorId : undefined}
              />
            </span>
          </label>
          {branchError && (
            <div className={styles.fieldControl}>
              <FieldError className={styles.fieldError} id={branchErrorId}>
                {branchError}
              </FieldError>
            </div>
          )}
        </div>

        <div className={styles.field}>
          <span className={kit.section}>Base branch</span>
          <div className={styles.fieldControl}>
            <div className={styles.branchPicker}>
              <FieldText
                aria-label="Base branch"
                value={baseBranch}
                placeholder="Current branch unavailable"
                onFocus={() => {
                  setBaseBranchSearchTouched(false);
                  setBaseBranchPickerOpen(true);
                }}
                onBlur={() => {
                  window.setTimeout(() => setBaseBranchPickerOpen(false), 120);
                }}
                onChange={(value) => {
                  setBaseBranch(value);
                  setBaseBranchSearchTouched(true);
                  setBaseBranchPickerOpen(true);
                }}
                disabled={isCreating}
              />
            </div>
            <span className={styles.helpText}>
              Worktree will be created from this branch.
            </span>
          </div>
          {baseBranchPickerOpen && filteredBaseOptions.length > 0 && (
            <div className={styles.branchOptions} role="listbox">
              {filteredBaseOptions.map((branch) => (
                <KitRow
                  key={branch}
                  label={branch}
                  selected={branch === baseBranch}
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => {
                    setBaseBranch(branch);
                    setBaseBranchSearchTouched(false);
                    setBaseBranchPickerOpen(false);
                  }}
                />
              ))}
            </div>
          )}
        </div>
      </div>
    </KitDialog>
  );
};

CreateWorktreeModal.displayName = "CreateWorktreeModal";
