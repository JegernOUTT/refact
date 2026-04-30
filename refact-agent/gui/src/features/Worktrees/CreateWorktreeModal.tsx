import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Button, Dialog, Flex, Text, TextField } from "@radix-ui/themes";
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
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Content className={styles.createDialog} maxWidth="420px">
        <Dialog.Title>Create worktree</Dialog.Title>
        <Dialog.Description size="2" color="gray">
          Create a new git worktree and attach it to this chat.
        </Dialog.Description>

        <div className={styles.modalFields}>
          <label className={styles.field} htmlFor="worktree-branch-name">
            <Text size="2" weight="medium">
              Branch name
            </Text>
            <TextField.Root
              id="worktree-branch-name"
              value={branchName}
              placeholder={defaultBranch}
              onChange={(event) => setBranchName(event.target.value)}
              disabled={isCreating}
            />
          </label>

          <div className={styles.field}>
            <Text size="2" weight="medium">
              Base branch
            </Text>
            <Text size="1" color="gray">
              Worktree will be created from this branch.
            </Text>
            <div className={styles.branchPicker}>
              <TextField.Root
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
                onChange={(event) => {
                  setBaseBranch(event.target.value);
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

          {error && (
            <Text size="1" color="red">
              {error}
            </Text>
          )}
        </div>

        <Flex className={styles.modalActions}>
          <Dialog.Close>
            <Button
              type="button"
              variant="soft"
              color="gray"
              disabled={isCreating}
            >
              Cancel
            </Button>
          </Dialog.Close>
          <Button
            type="button"
            onClick={() => void handleCreate()}
            disabled={!canCreate}
          >
            {isCreating ? "Creating..." : "Create"}
          </Button>
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};

CreateWorktreeModal.displayName = "CreateWorktreeModal";
