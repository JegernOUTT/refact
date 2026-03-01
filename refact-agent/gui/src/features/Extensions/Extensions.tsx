import React, { useState, useCallback } from "react";
import { Flex, Button, Tabs } from "@radix-ui/themes";
import { ArrowLeftIcon } from "@radix-ui/react-icons";

import { PageWrapper } from "../../components/PageWrapper";
import { ScrollArea } from "../../components/ScrollArea";
import { Spinner } from "../../components/Spinner";
import type { Config } from "../Config/configSlice";
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
import { MarketplacePanel } from "./components/MarketplacePanel";

import styles from "./Extensions.module.css";

export type ExtensionsTab = "skills" | "commands" | "hooks" | "marketplace";

export type ExtensionsProps = {
  backFromExtensions: () => void;
  host: Config["host"];
  tabbed: Config["tabbed"];
  initialTab?: ExtensionsTab;
  initialItemId?: string;
};

export const Extensions: React.FC<ExtensionsProps> = ({
  backFromExtensions,
  host,
  tabbed,
  initialTab = "skills",
  initialItemId,
}) => {
  const [activeTab, setActiveTab] = useState<ExtensionsTab>(initialTab);
  const [selectedSkill, setSelectedSkill] = useState<string | null>(initialItemId ?? null);
  const [selectedCommand, setSelectedCommand] = useState<string | null>(null);
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [createDialogType, setCreateDialogType] = useState<"skill" | "command">("skill");

  const { data: registry, isLoading, refetch } = useGetExtRegistryQuery(undefined);
  const [deleteSkill] = useDeleteSkillMutation();
  const [deleteCommand] = useDeleteCommandMutation();

  const handleTabChange = useCallback((value: string) => {
    setActiveTab(value as ExtensionsTab);
    setSelectedSkill(null);
    setSelectedCommand(null);
  }, []);

  const handleDeleteSkill = useCallback(
    async (name: string) => {
      if (!confirm(`Delete skill "${name}"?`)) return;
      await deleteSkill({ name });
      if (selectedSkill === name) setSelectedSkill(null);
      await refetch();
    },
    [selectedSkill, deleteSkill, refetch],
  );

  const handleDeleteCommand = useCallback(
    async (name: string) => {
      if (!confirm(`Delete command "${name}"?`)) return;
      await deleteCommand({ name });
      if (selectedCommand === name) setSelectedCommand(null);
      await refetch();
    },
    [selectedCommand, deleteCommand, refetch],
  );

  const openCreateDialog = useCallback((type: "skill" | "command") => {
    setCreateDialogType(type);
    setCreateDialogOpen(true);
  }, []);

  const hasProjectRoot =
    registry !== undefined &&
    (registry.skills.some((s) => s.scope === "local") ||
      registry.slash_commands.some((c) => c.scope === "local"));

  if (isLoading) return <Spinner spinning />;

  return (
    <PageWrapper host={host} noPadding>
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
          <Tabs.Trigger value="marketplace">Marketplace</Tabs.Trigger>
        </Tabs.List>

        <div className={styles.panelContainer}>
          <Tabs.Content value="skills" style={{ height: "100%", display: "flex", flexDirection: "column" }}>
            {selectedSkill ? (
              <div className={styles.editorPanel}>
                <SkillEditor
                  name={selectedSkill}
                  onBack={() => setSelectedSkill(null)}
                />
              </div>
            ) : (
              <ScrollArea scrollbars="vertical" className={styles.listPanel}>
                <ExtItemList
                  items={registry?.skills ?? []}
                  selectedId={selectedSkill}
                  onSelect={setSelectedSkill}
                  onCreate={() => openCreateDialog("skill")}
                  onDelete={(name) => void handleDeleteSkill(name)}
                />
              </ScrollArea>
            )}
          </Tabs.Content>

          <Tabs.Content value="commands" style={{ height: "100%", display: "flex", flexDirection: "column" }}>
            {selectedCommand ? (
              <div className={styles.editorPanel}>
                <CommandEditor
                  name={selectedCommand}
                  onBack={() => setSelectedCommand(null)}
                />
              </div>
            ) : (
              <ScrollArea scrollbars="vertical" className={styles.listPanel}>
                <ExtItemList
                  items={registry?.slash_commands ?? []}
                  selectedId={selectedCommand}
                  onSelect={setSelectedCommand}
                  onCreate={() => openCreateDialog("command")}
                  onDelete={(name) => void handleDeleteCommand(name)}
                />
              </ScrollArea>
            )}
          </Tabs.Content>

          <Tabs.Content value="hooks" style={{ height: "100%", display: "flex", flexDirection: "column", overflow: "auto" }}>
            <HooksEditor />
          </Tabs.Content>

          <Tabs.Content value="marketplace" style={{ height: "100%", display: "flex", flexDirection: "column" }}>
            <MarketplacePanel />
          </Tabs.Content>
        </div>
      </Tabs.Root>

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
    </PageWrapper>
  );
};
