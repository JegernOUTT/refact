import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Button, Dialog, FieldText } from "../../components/ui";
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
    <Dialog open={open} onOpenChange={onOpenChange}>
      <Dialog.Content className={styles.createDialog} maxWidth="420px">
        <div {...dialogNonInteractiveCloseHandlers(() => onOpenChange(false))}>
          <Dialog.Title>Create worktree</Dialog.Title>
          <Dialog.Description>
            Create a new git worktree and attach it to this chat.
          </Dialog.Description>

          <div className={styles.modalFields}>
            <label className={styles.field} htmlFor="worktree-branch-name">
              <span className={styles.labelText}>Branch name</span>
              <FieldText
                id="worktree-branch-name"
                value={branchName}
                placeholder={defaultBranch}
                onChange={setBranchName}
                disabled={isCreating}
              />
            </label>

            <div className={styles.field}>
              <span className={styles.labelText}>Base branch</span>
              <span className={styles.helpText}>
                Worktree will be created from this branch.
              </span>
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
                {baseBranchPickerOpen && filteredBaseOptions.length > 0 && (
                  <div className={styles.branchOptions} role="listbox">
                    {filteredBaseOptions.map((branch) => (
                      <button
                        key={branch}
                        type="button"
                        className={styles.branchOption}
                        onMouseDown={(event) => event.preventDefault()}
                        onClick={() => {
                          setBaseBranch(branch);
                          setBaseBranchSearchTouched(false);
                          setBaseBranchPickerOpen(false);
                        }}
                      >
                        {branch}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            </div>

            {error && <p className={styles.errorText}>{error}</p>}
          </div>

          <div className={styles.modalActions}>
            <Dialog.Close asChild>
              <Button variant="soft" disabled={isCreating}>
                Cancel
              </Button>
            </Dialog.Close>
            <Button
              variant="primary"
              onClick={() => void handleCreate()}
              disabled={!canCreate}
              loading={isCreating}
            >
              {isCreating ? "Creating..." : "Create"}
            </Button>
          </div>
        </div>
      </Dialog.Content>
    </Dialog>
  );
};

CreateWorktreeModal.displayName = "CreateWorktreeModal";
