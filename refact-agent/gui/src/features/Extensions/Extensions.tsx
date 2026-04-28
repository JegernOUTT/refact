import React, { useState, useCallback } from "react";
import { AlertDialog, Flex, Button, Text, Tabs } from "@radix-ui/themes";
import { ArrowLeftIcon } from "@radix-ui/react-icons";

import { PageWrapper } from "../../components/PageWrapper";
import { Spinner } from "../../components/Spinner";
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

export type ExtensionsTab = "skills" | "commands" | "hooks";

export type ExtensionsProps = {
  backFromExtensions: () => void;
  host: Config["host"];
  tabbed: Config["tabbed"];
  initialTab?: ExtensionsTab;
  initialItemId?: string;
  draftId?: string;
};

type DeleteTarget = {
  type: "skill" | "command";
  name: string;
  scope: "global" | "local" | "plugin";
};

export const Extensions: React.FC<ExtensionsProps> = ({
  backFromExtensions,
  host,
  tabbed,
  initialTab = "skills",
  initialItemId,
  draftId,
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
    dispatch(push({ name: "skills marketplace" }));
  }, [dispatch]);

  const openCommandsMarketplace = useCallback(() => {
    dispatch(push({ name: "commands marketplace" }));
  }, [dispatch]);

  const hasProjectRoot = registry?.has_project_root ?? false;

  if (isLoading) return <Spinner spinning />;

  if (isError) {
    return (
      <PageWrapper host={host} noPadding>
        <Flex direction="column" align="center" gap="3" p="4">
          <Text color="red">Failed to load extensions registry</Text>
          <Button onClick={() => void refetch()}>Retry</Button>
        </Flex>
      </PageWrapper>
    );
  }

  return (
    <PageWrapper host={host} noPadding>
      <div className={styles.page}>
        {host === "vscode" && !tabbed ? (
          <Flex gap="2" pb="2">
            <Button variant="surface" onClick={backFromExtensions}>
              <ArrowLeftIcon width="16" height="16" />
              Back
            </Button>
          </Flex>
        ) : (
          <Button
            mr="auto"
            variant="outline"
            onClick={backFromExtensions}
            mb="2"
          >
            Back
          </Button>
        )}

        <Tabs.Root value={activeTab} onValueChange={handleTabChange}>
          <Tabs.List size="1">
            <Tabs.Trigger value="skills">
              Skills ({registry?.skills.length ?? 0})
            </Tabs.Trigger>
            <Tabs.Trigger value="commands">
              Commands ({registry?.slash_commands.length ?? 0})
            </Tabs.Trigger>
            <Tabs.Trigger value="hooks">Hooks</Tabs.Trigger>
          </Tabs.List>
        </Tabs.Root>

        {deleteError && (
          <Text color="red" size="2" mt="2">
            {deleteError}
          </Text>
        )}

        <div className={styles.panelContainer}>
          {activeTab === "skills" &&
            (selectedSkill ? (
              <SkillEditor
                name={selectedSkill}
                onBack={() => setSelectedSkill(null)}
                draftId={draftId}
              />
            ) : (
              <Flex direction="column" gap="2">
                <Button
                  variant="outline"
                  size="1"
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
              </Flex>
            ))}

          {activeTab === "commands" &&
            (selectedCommand ? (
              <CommandEditor
                name={selectedCommand}
                onBack={() => setSelectedCommand(null)}
                draftId={draftId}
              />
            ) : (
              <Flex direction="column" gap="2">
                <Button
                  variant="outline"
                  size="1"
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
              </Flex>
            ))}

          {activeTab === "hooks" && <HooksEditor />}
        </div>

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

        <AlertDialog.Root
          open={deleteTarget !== null}
          onOpenChange={(open) => {
            if (!open) setDeleteTarget(null);
          }}
        >
          <AlertDialog.Content maxWidth="400px">
            <AlertDialog.Title>Confirm Delete</AlertDialog.Title>
            <AlertDialog.Description>
              {`Delete ${deleteTarget?.type ?? ""} "${
                deleteTarget?.name ?? ""
              }"?`}
            </AlertDialog.Description>
            <Flex gap="3" mt="4" justify="end">
              <AlertDialog.Cancel>
                <Button variant="soft" color="gray">
                  Cancel
                </Button>
              </AlertDialog.Cancel>
              <AlertDialog.Action>
                <Button color="red" onClick={() => void confirmDelete()}>
                  Delete
                </Button>
              </AlertDialog.Action>
            </Flex>
          </AlertDialog.Content>
        </AlertDialog.Root>
      </div>
    </PageWrapper>
  );
};
