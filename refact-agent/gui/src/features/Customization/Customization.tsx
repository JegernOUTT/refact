import React, { useState, useCallback } from "react";
import { Flex, Button, Tabs, Text, Card, Badge, IconButton, Dialog, TextField } from "@radix-ui/themes";
import { ArrowLeftIcon, PlusIcon, TrashIcon } from "@radix-ui/react-icons";

import { ScrollArea } from "../../components/ScrollArea";
import { PageWrapper } from "../../components/PageWrapper";
import { Spinner } from "../../components/Spinner";
import {
  useGetRegistryQuery,
  useGetConfigQuery,
  useSaveConfigMutation,
  useCreateConfigMutation,
  useDeleteConfigMutation,
  ConfigItem,
  ConfigKind,
} from "../../services/refact/customization";
import type { Config } from "../Config/configSlice";

import styles from "./Customization.module.css";

export type CustomizationProps = {
  backFromCustomization: () => void;
  host: Config["host"];
  tabbed: Config["tabbed"];
  initialKind?: ConfigKind;
  initialConfigId?: string;
};

const KIND_LABELS: Record<ConfigKind, string> = {
  modes: "Modes",
  subagents: "Subagents",
  toolbox_commands: "Toolbox",
  code_lens: "Code Lens",
};

const ConfigList: React.FC<{
  items: ConfigItem[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onDelete: (id: string) => void;
  onCreate: () => void;
}> = ({ items, selectedId, onSelect, onDelete, onCreate }) => {
  return (
    <Flex direction="column" gap="2" className={styles.configList}>
      <Button variant="soft" onClick={onCreate} size="1">
        <PlusIcon /> New
      </Button>
      {items.map((item) => (
        <Card
          key={item.id}
          role="button"
          tabIndex={0}
          className={`${styles.configItem} ${selectedId === item.id ? styles.selected : ""}`}
          onClick={() => onSelect(item.id)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              onSelect(item.id);
            }
          }}
        >
          <Flex justify="between" align="center">
            <Flex direction="column" gap="1">
              <Text size="2" weight="medium">{item.title}</Text>
              <Text size="1" color="gray">{item.id}</Text>
            </Flex>
            <Flex gap="1" align="center">
              {item.specific && <Badge size="1" color="gray">internal</Badge>}
              <IconButton
                size="1"
                variant="ghost"
                color="red"
                onClick={(e) => {
                  e.stopPropagation();
                  onDelete(item.id);
                }}
              >
                <TrashIcon />
              </IconButton>
            </Flex>
          </Flex>
        </Card>
      ))}
      {items.length === 0 && (
        <Text size="2" color="gray">No configs found</Text>
      )}
    </Flex>
  );
};

const ConfigEditor: React.FC<{
  kind: ConfigKind;
  configId: string;
  onSaved: () => void;
}> = ({ kind, configId, onSaved }) => {
  const { data, isLoading, error } = useGetConfigQuery({ kind, id: configId });
  const [saveConfig, { isLoading: isSaving }] = useSaveConfigMutation();
  const [yaml, setYaml] = useState<string>("");
  const [saveError, setSaveError] = useState<string | null>(null);

  React.useEffect(() => {
    if (data?.raw_yaml) {
      setYaml(data.raw_yaml);
    }
  }, [data?.raw_yaml]);

  const handleSave = useCallback(async () => {
    setSaveError(null);
    try {
      const config = await import("js-yaml").then((m) => m.load(yaml) as Record<string, unknown>);
      const result = await saveConfig({ kind, id: configId, config }).unwrap();
      if (!result.ok && result.errors.length > 0) {
        setSaveError(result.errors.map((e) => e.error).join(", "));
      } else {
        onSaved();
      }
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    }
  }, [yaml, kind, configId, saveConfig, onSaved]);

  if (isLoading) return <Spinner spinning />;
  if (error) return <Text color="red">Error loading config</Text>;

  return (
    <Flex direction="column" gap="3" className={styles.configEditor}>
      <Flex justify="between" align="center">
        <Text size="3" weight="bold">{configId}</Text>
        <Button onClick={handleSave} disabled={isSaving}>
          {isSaving ? "Saving..." : "Save"}
        </Button>
      </Flex>
      {saveError && <Text size="2" color="red">{saveError}</Text>}
      <Text size="1" color="gray">{data?.file_path}</Text>
      <textarea
        className={styles.yamlEditor}
        value={yaml}
        onChange={(e) => setYaml(e.target.value)}
        spellCheck={false}
      />
    </Flex>
  );
};

const CreateConfigDialog: React.FC<{
  kind: ConfigKind;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (id: string) => void;
}> = ({ kind, open, onOpenChange, onCreated }) => {
  const [id, setId] = useState("");
  const [createConfig, { isLoading }] = useCreateConfigMutation();
  const [error, setError] = useState<string | null>(null);

  const handleCreate = useCallback(async () => {
    setError(null);
    if (!id.trim()) {
      setError("ID is required");
      return;
    }
    const defaultConfig = getDefaultConfig(kind, id);
    try {
      const result = await createConfig({ kind, id, config: defaultConfig }).unwrap();
      if (!result.ok && result.errors.length > 0) {
        setError(result.errors.map((e) => e.error).join(", "));
      } else {
        setId("");
        onOpenChange(false);
        onCreated(id);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [kind, id, createConfig, onOpenChange, onCreated]);

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Content style={{ maxWidth: 400 }}>
        <Dialog.Title>Create {KIND_LABELS[kind]}</Dialog.Title>
        <Flex direction="column" gap="3">
          <TextField.Root
            placeholder="Config ID (e.g., my_mode)"
            value={id}
            onChange={(e) => setId(e.target.value)}
          />
          {error && <Text size="2" color="red">{error}</Text>}
        </Flex>
        <Flex gap="3" mt="4" justify="end">
          <Dialog.Close>
            <Button variant="soft" color="gray">Cancel</Button>
          </Dialog.Close>
          <Button onClick={handleCreate} disabled={isLoading}>
            {isLoading ? "Creating..." : "Create"}
          </Button>
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};

function getDefaultConfig(kind: ConfigKind, id: string): Record<string, unknown> {
  switch (kind) {
    case "modes":
      return {
        schema_version: 1,
        id,
        title: id,
        description: "",
        specific: false,
        prompt: "",
        tools: [],
      };
    case "subagents":
      return {
        schema_version: 1,
        id,
        title: id,
        description: "",
        specific: false,
        expose_as_tool: true,
        has_code: false,
        subchat: { context_mode: "bare" },
        messages: {},
      };
    case "toolbox_commands":
      return {
        schema_version: 1,
        id,
        description: "",
        messages: [],
      };
    case "code_lens":
      return {
        schema_version: 1,
        id,
        label: id,
        auto_submit: false,
        new_tab: false,
        messages: [],
      };
  }
}

export const Customization: React.FC<CustomizationProps> = ({
  backFromCustomization,
  host,
  tabbed,
  initialKind = "modes",
  initialConfigId,
}) => {
  const [activeKind, setActiveKind] = useState<ConfigKind>(initialKind);
  const [selectedConfigId, setSelectedConfigId] = useState<string | null>(initialConfigId ?? null);
  const [createDialogOpen, setCreateDialogOpen] = useState(false);

  const { data: registry, isLoading, refetch } = useGetRegistryQuery();
  const [deleteConfig] = useDeleteConfigMutation();

  const getItemsForKind = (kind: ConfigKind): ConfigItem[] => {
    if (!registry) return [];
    switch (kind) {
      case "modes": return registry.modes;
      case "subagents": return registry.subagents;
      case "toolbox_commands": return registry.toolbox_commands;
      case "code_lens": return registry.code_lens;
    }
  };

  const handleDelete = useCallback(async (id: string) => {
    if (!confirm(`Delete ${id}?`)) return;
    await deleteConfig({ kind: activeKind, id });
    if (selectedConfigId === id) {
      setSelectedConfigId(null);
    }
    refetch();
  }, [activeKind, selectedConfigId, deleteConfig, refetch]);

  const handleTabChange = useCallback((value: string) => {
    setActiveKind(value as ConfigKind);
    setSelectedConfigId(null);
  }, []);

  if (isLoading) return <Spinner spinning />;

  return (
    <PageWrapper host={host} style={{ padding: 0, marginTop: 0 }}>
      {host === "vscode" && !tabbed ? (
        <Flex gap="2" pb="3">
          <Button variant="surface" onClick={backFromCustomization}>
            <ArrowLeftIcon width="16" height="16" />
            Back
          </Button>
        </Flex>
      ) : (
        <Button mr="auto" variant="outline" onClick={backFromCustomization} mb="4">
          Back
        </Button>
      )}

      {registry?.errors && registry.errors.length > 0 && (
        <Card mb="3" style={{ backgroundColor: "var(--red-3)" }}>
          <Text size="2" color="red">
            {registry.errors.length} config error(s): {registry.errors.map((e) => e.error).join(", ")}
          </Text>
        </Card>
      )}

      <Tabs.Root value={activeKind} onValueChange={handleTabChange}>
        <Tabs.List>
          {(Object.keys(KIND_LABELS) as ConfigKind[]).map((kind) => (
            <Tabs.Trigger key={kind} value={kind}>
              {KIND_LABELS[kind]} ({getItemsForKind(kind).length})
            </Tabs.Trigger>
          ))}
        </Tabs.List>

        <Flex gap="4" mt="4" style={{ height: "calc(100vh - 200px)" }}>
          <ScrollArea scrollbars="vertical" style={{ width: 280 }}>
            <ConfigList
              items={getItemsForKind(activeKind)}
              selectedId={selectedConfigId}
              onSelect={setSelectedConfigId}
              onDelete={handleDelete}
              onCreate={() => setCreateDialogOpen(true)}
            />
          </ScrollArea>

          <ScrollArea scrollbars="vertical" style={{ flex: 1 }}>
            {selectedConfigId ? (
              <ConfigEditor
                kind={activeKind}
                configId={selectedConfigId}
                onSaved={() => refetch()}
              />
            ) : (
              <Flex align="center" justify="center" style={{ height: "100%" }}>
                <Text color="gray">Select a config to edit</Text>
              </Flex>
            )}
          </ScrollArea>
        </Flex>
      </Tabs.Root>

      <CreateConfigDialog
        kind={activeKind}
        open={createDialogOpen}
        onOpenChange={setCreateDialogOpen}
        onCreated={(id) => setSelectedConfigId(id)}
      />
    </PageWrapper>
  );
};
