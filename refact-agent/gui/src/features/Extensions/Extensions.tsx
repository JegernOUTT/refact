import React, { useState, useCallback } from "react";
import { ArrowLeft } from "lucide-react";

import { PageWrapper } from "../../components/PageWrapper";
import {
  Button,
  Dialog,
  EmptyState,
  FieldError,
  LoadingState,
  SegmentedControl,
} from "../../components/ui";
import type { Config } from "../Config/configSlice";
import { useAppDispatch } from "../../hooks";
import {
  useGetExtRegistryQuery,
  useDeleteSkillMutation,
  useDeleteCommandMutation,
} from "../../services/refact/extensions";
import {
  ExtItemList,
  SkillEditor,
  CommandEditor,
  HooksEditor,
  CreateItemDialog,
} from "./components";
import styles from "./Extensions.module.css";
import { push } from "../Pages/pagesSlice";
import { SettingsSection } from "../Settings/SettingsSection";

export type ExtensionsTab = "skills" | "commands" | "hooks";

export type ExtensionsProps = {
  backFromExtensions: () => void;
  host: Config["host"];
  tabbed: Config["tabbed"];
  initialTab?: ExtensionsTab;
  initialItemId?: string;
  draftId?: string;
  embedded?: boolean;
};

type DeleteTarget = {
  type: "skill" | "command";
  name: string;
  scope: "global" | "local" | "plugin";
};

export const Extensions: React.FC<ExtensionsProps> = ({
  backFromExtensions,
  host,
  initialTab = "skills",
  initialItemId,
  draftId,
  embedded = false,
}) => {
  const dispatch = useAppDispatch();
  const [activeTab, setActiveTab] = useState<ExtensionsTab>(initialTab);
  const [selectedSkill, setSelectedSkill] = useState<string | null>(
    initialTab === "skills" ? initialItemId ?? null : null,
  );
  const [selectedCommand, setSelectedCommand] = useState<string | null>(
    initialTab === "commands" ? initialItemId ?? null : null,
  );
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [createDialogType, setCreateDialogType] = useState<"skill" | "command">(
    "skill",
  );
  const [deleteTarget, setDeleteTarget] = useState<DeleteTarget | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  const {
    data: registry,
    isLoading,
    isError,
    refetch,
  } = useGetExtRegistryQuery(undefined);
  const [deleteSkill] = useDeleteSkillMutation();
  const [deleteCommand] = useDeleteCommandMutation();

  const handleTabChange = useCallback((value: string) => {
    setActiveTab(value as ExtensionsTab);
    setSelectedSkill(null);
    setSelectedCommand(null);
  }, []);

  const handleDeleteSkill = useCallback(
    (name: string, scope: "global" | "local" | "plugin") => {
      setDeleteError(null);
      setDeleteTarget({ type: "skill", name, scope });
    },
    [],
  );

  const handleDeleteCommand = useCallback(
    (name: string, scope: "global" | "local" | "plugin") => {
      setDeleteError(null);
      setDeleteTarget({ type: "command", name, scope });
    },
    [],
  );

  const confirmDelete = useCallback(async () => {
    if (!deleteTarget) return;
    const { type, name, scope } = deleteTarget;
    try {
      if (type === "skill") {
        await deleteSkill({ name, scope }).unwrap();
        if (selectedSkill === name) setSelectedSkill(null);
      } else {
        await deleteCommand({ name, scope }).unwrap();
        if (selectedCommand === name) setSelectedCommand(null);
      }
      await refetch();
    } catch (err: unknown) {
      const message =
        err && typeof err === "object" && "data" in err
          ? String((err as { data: unknown }).data)
          : "Delete failed";
      setDeleteError(message);
    }
    setDeleteTarget(null);
  }, [
    deleteTarget,
    deleteSkill,
    deleteCommand,
    selectedSkill,
    selectedCommand,
    refetch,
  ]);

  const openCreateDialog = useCallback((type: "skill" | "command") => {
    setCreateDialogType(type);
    setCreateDialogOpen(true);
  }, []);

  const openSkillsMarketplace = useCallback(() => {
    dispatch(push({ name: "marketplace hub", tab: "skills" }));
  }, [dispatch]);

  const openCommandsMarketplace = useCallback(() => {
    dispatch(push({ name: "marketplace hub", tab: "commands" }));
  }, [dispatch]);

  const hasProjectRoot = registry?.has_project_root ?? false;

  if (isLoading) {
    const loadingContent = (
      <SettingsSection
        title="Extensions"
        description="Manage reusable skills, slash commands, and automation hooks."
      >
        <LoadingState label="Loading extensions registry" />
      </SettingsSection>
    );
    return embedded ? (
      loadingContent
    ) : (
      <PageWrapper host={host} noPadding>
        {loadingContent}
      </PageWrapper>
    );
  }

  if (isError) {
    const errorContent = (
      <SettingsSection
        title="Extensions"
        description="Manage reusable skills, slash commands, and automation hooks."
      >
        <EmptyState
          action={<Button onClick={() => void refetch()}>Retry</Button>}
          title="Failed to load extensions registry"
          variant="full"
        />
      </SettingsSection>
    );
    return embedded ? (
      errorContent
    ) : (
      <PageWrapper host={host} noPadding>
        {errorContent}
      </PageWrapper>
    );
  }

  const tabs = (
    <SegmentedControl
      className={styles.tabs}
      value={activeTab}
      onValueChange={handleTabChange}
      size="sm"
      options={[
        {
          value: "skills",
          label: (
            <span className={styles.tabLabel}>
              Skills <span>({registry?.skills.length ?? 0})</span>
            </span>
          ),
        },
        {
          value: "commands",
          label: (
            <span className={styles.tabLabel}>
              Commands <span>({registry?.slash_commands.length ?? 0})</span>
            </span>
          ),
        },
        { value: "hooks", label: "Hooks" },
      ]}
    />
  );

  const actions = !embedded ? (
    <Button variant="soft" onClick={backFromExtensions} leftIcon={ArrowLeft}>
      Back
    </Button>
  ) : null;

  const inner = (
    <>
      <SettingsSection
        title="Extensions"
        description="Manage reusable skills, slash commands, and automation hooks."
        actions={actions}
        subNav={tabs}
        width="wide"
      >
        {deleteError ? <FieldError>{deleteError}</FieldError> : null}

        <div className={styles.panelContainer}>
          {activeTab === "skills" &&
            (selectedSkill ? (
              <SkillEditor
                name={selectedSkill}
                onBack={() => setSelectedSkill(null)}
                draftId={draftId}
              />
            ) : (
              <div className={`${styles.actionsStack} rf-stagger`}>
                <Button
                  variant="soft"
                  size="sm"
                  onClick={openSkillsMarketplace}
                >
                  Browse Skills Marketplace
                </Button>
                <ExtItemList
                  items={registry?.skills ?? []}
                  selectedId={selectedSkill}
                  onSelect={setSelectedSkill}
                  onCreate={() => openCreateDialog("skill")}
                  onDelete={handleDeleteSkill}
                />
              </div>
            ))}

          {activeTab === "commands" &&
            (selectedCommand ? (
              <CommandEditor
                name={selectedCommand}
                onBack={() => setSelectedCommand(null)}
                draftId={draftId}
              />
            ) : (
              <div className={`${styles.actionsStack} rf-stagger`}>
                <Button
                  variant="soft"
                  size="sm"
                  onClick={openCommandsMarketplace}
                >
                  Browse Commands Marketplace
                </Button>
                <ExtItemList
                  items={registry?.slash_commands ?? []}
                  selectedId={selectedCommand}
                  onSelect={setSelectedCommand}
                  onCreate={() => openCreateDialog("command")}
                  onDelete={handleDeleteCommand}
                />
              </div>
            ))}

          {activeTab === "hooks" && <HooksEditor />}
        </div>
      </SettingsSection>

      <CreateItemDialog
        type={createDialogType}
        open={createDialogOpen}
        onOpenChange={setCreateDialogOpen}
        onCreated={(name) => {
          if (createDialogType === "skill") setSelectedSkill(name);
          else setSelectedCommand(name);
          void refetch();
        }}
        hasProjectRoot={hasProjectRoot}
      />

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
      >
        <Dialog.Content maxWidth="400px">
          <Dialog.Title>Confirm Delete</Dialog.Title>
          <Dialog.Description>
            {`Delete ${deleteTarget?.type ?? ""} "${
              deleteTarget?.name ?? ""
            }"?`}
          </Dialog.Description>
          <div className={styles.dialogActions}>
            <Dialog.Close asChild>
              <Button variant="soft">Cancel</Button>
            </Dialog.Close>
            <Dialog.Close asChild>
              <Button variant="danger" onClick={() => void confirmDelete()}>
                Delete
              </Button>
            </Dialog.Close>
          </div>
        </Dialog.Content>
      </Dialog>
    </>
  );

  if (embedded) return inner;
  return (
    <PageWrapper host={host} noPadding>
      {inner}
    </PageWrapper>
  );
};
