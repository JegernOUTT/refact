import React, { useState, useCallback, useRef, useEffect } from "react";
import {
  Flex,
  Button,
  Tabs,
  Text,
  Badge,
  IconButton,
  Dialog,
  TextField,
  SegmentedControl,
  Card,
  Callout,
} from "@radix-ui/themes";
import {
  ArrowLeftIcon,
  PlusIcon,
  TrashIcon,
  GlobeIcon,
  FileIcon,
  CodeIcon,
  MixerHorizontalIcon,
  ExternalLinkIcon,
  InfoCircledIcon,
} from "@radix-ui/react-icons";
import { skipToken } from "@reduxjs/toolkit/query";

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
import { useGetDraftQuery } from "../../services/refact/buddy";
import type { Config } from "../Config/configSlice";
import {
  CodeLensForm,
  ToolboxCommandForm,
  ModeForm,
  SubagentForm,
} from "./components";
import {
  applyPatch,
  isPlainObject,
  sanitizeObject,
  ConfigPatch,
  validateConfigId,
} from "./components/configUtils";
import { useAppDispatch } from "../../hooks";
import { push } from "../Pages/pagesSlice";
import { BuddyDraftPreview } from "../Buddy/BuddyDraftPreview";

import styles from "./Customization.module.css";

export type CustomizationProps = {
  backFromCustomization: () => void;
  host: Config["host"];
  tabbed: Config["tabbed"];
  initialKind?: ConfigKind;
  initialConfigId?: string;
  draftId?: string;
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
  onDelete: (id: string, scope: "global" | "local") => void;
  onCreate: () => void;
}> = ({ items, selectedId, onSelect, onDelete, onCreate }) => {
  return (
    <Flex direction="column" gap="1" className={styles.configList}>
      <Button variant="soft" onClick={onCreate} size="1">
        <PlusIcon /> New
      </Button>
      {items.map((item) => (
        <div
          key={item.id}
          role="button"
          tabIndex={0}
          className={`${styles.compactConfigItem} ${
            selectedId === item.id ? styles.selected : ""
          }`}
          onClick={() => onSelect(item.id)}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              onSelect(item.id);
            }
          }}
        >
          <Flex direction="column" gap="0" style={{ minWidth: 0, flex: 1 }}>
            <Text
              size="1"
              weight="medium"
              style={{
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {item.title}
            </Text>
            <Flex align="center" gap="1">
              <Text
                size="1"
                color="gray"
                style={{
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
              >
                {item.id}
              </Text>
              <Badge
                size="1"
                color={item.scope === "global" ? "blue" : "green"}
                variant="soft"
              >
                {item.scope === "global" ? "G" : "L"}
              </Badge>
            </Flex>
          </Flex>
          <IconButton
            size="1"
            variant="ghost"
            color="red"
            onClick={(e) => {
              e.stopPropagation();
              onDelete(item.id, item.scope);
            }}
          >
            <TrashIcon />
          </IconButton>
        </div>
      ))}
      {items.length === 0 && (
        <Text size="1" color="gray">
          No configs found
        </Text>
      )}
    </Flex>
  );
};

type EditorView = "form" | "yaml";

const jsYamlPromise = import("js-yaml");

export const ConfigEditor: React.FC<{
  kind: ConfigKind;
  configId: string;
  configItem: ConfigItem;
  onSaved: () => void;
  draftId?: string;
}> = ({ kind, configId, configItem, onSaved, draftId }) => {
  const { data, isLoading, error } = useGetConfigQuery({ kind, id: configId });
  const {
    data: draft,
    isLoading: draftLoading,
    error: draftError,
  } = useGetDraftQuery(draftId ?? skipToken);
  const [saveConfig, { isLoading: isSaving }] = useSaveConfigMutation();
  const [configJson, setConfigJson] = useState<Record<string, unknown> | null>(
    null,
  );
  const [yaml, setYaml] = useState<string>("");
  const [saveError, setSaveError] = useState<string | null>(null);
  const [draftExpired, setDraftExpired] = useState(false);
  const [targetScope, setTargetScope] = useState<"global" | "local">(
    configItem.scope,
  );
  const [view, setView] = useState<EditorView>("form");
  const [yamlParseError, setYamlParseError] = useState<string | null>(null);
  const yamlSyncTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const syncVersionRef = useRef(0);

  useEffect(() => {
    if (draftError) {
      setDraftExpired(true);
    }
  }, [draftError]);

  useEffect(() => {
    if (draft) {
      const version = ++syncVersionRef.current;
      void (async () => {
        try {
          const jsYaml = await jsYamlPromise;
          if (version !== syncVersionRef.current) return;
          const parsed = jsYaml.load(draft.yaml_or_json);
          if (isPlainObject(parsed)) {
            const sanitized = sanitizeObject(parsed) as Record<string, unknown>;
            setConfigJson(sanitized);
            setYaml(draft.yaml_or_json);
            setYamlParseError(null);
          }
        } catch {
          // ignore parse error; fall back to server data
        }
      })();
    }
  }, [draft]);

  useEffect(() => {
    if (data && !draft) {
      if (yamlSyncTimeoutRef.current) {
        clearTimeout(yamlSyncTimeoutRef.current);
        yamlSyncTimeoutRef.current = null;
      }
      syncVersionRef.current++;
      setConfigJson(data.config);
      setYaml(data.raw_yaml);
      setYamlParseError(null);
    }
  }, [data, draft]);

  useEffect(() => {
    const versionRef = syncVersionRef;
    return () => {
      if (yamlSyncTimeoutRef.current) {
        clearTimeout(yamlSyncTimeoutRef.current);
      }
      versionRef.current++;
    };
  }, []);

  useEffect(() => {
    setTargetScope(configItem.scope);
  }, [configItem.scope]);

  const syncYamlToJson = useCallback(
    async (yamlStr: string, version: number) => {
      try {
        const jsYaml = await jsYamlPromise;
        if (version !== syncVersionRef.current) return;
        const parsed = jsYaml.load(yamlStr);
        if (!isPlainObject(parsed)) {
          setYamlParseError("Config must be an object");
          return;
        }
        const sanitized = sanitizeObject(parsed) as Record<string, unknown>;
        setConfigJson(sanitized);
        setYamlParseError(null);
      } catch (e) {
        if (version !== syncVersionRef.current) return;
        setYamlParseError(e instanceof Error ? e.message : String(e));
      }
    },
    [],
  );

  const syncJsonToYaml = useCallback(
    async (json: Record<string, unknown>, version: number) => {
      try {
        const jsYaml = await jsYamlPromise;
        if (version !== syncVersionRef.current) return;
        const yamlStr = jsYaml.dump(json, {
          indent: 2,
          lineWidth: -1,
          noRefs: true,
        });
        setYaml(yamlStr);
        setYamlParseError(null);
      } catch (e) {
        if (version !== syncVersionRef.current) return;
        setYamlParseError(e instanceof Error ? e.message : String(e));
      }
    },
    [],
  );

  const handleYamlChange = useCallback(
    (yamlStr: string) => {
      setYaml(yamlStr);
      if (yamlSyncTimeoutRef.current) clearTimeout(yamlSyncTimeoutRef.current);
      yamlSyncTimeoutRef.current = setTimeout(() => {
        const version = ++syncVersionRef.current;
        void syncYamlToJson(yamlStr, version);
      }, 300);
    },
    [syncYamlToJson],
  );

  const handleFormPatch = useCallback(
    (patch: ConfigPatch) => {
      setConfigJson((prev) => {
        if (!prev) return prev;
        const updated = applyPatch(prev, patch);
        if (yamlSyncTimeoutRef.current)
          clearTimeout(yamlSyncTimeoutRef.current);
        yamlSyncTimeoutRef.current = setTimeout(() => {
          const version = ++syncVersionRef.current;
          void syncJsonToYaml(updated, version);
        }, 300);
        return updated;
      });
    },
    [syncJsonToYaml],
  );

  const handleSave = useCallback(async () => {
    setSaveError(null);
    if (!configJson) {
      setSaveError("No config to save");
      return;
    }
    try {
      const result = await saveConfig({
        kind,
        id: configId,
        config: configJson,
        scope: targetScope,
        draft_id: draftId,
      }).unwrap();
      if (!result.ok && result.errors.length > 0) {
        setSaveError(result.errors.map((e) => e.error).join(", "));
      } else {
        onSaved();
      }
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    }
  }, [configJson, kind, configId, saveConfig, onSaved, targetScope, draftId]);

  if (isLoading || draftLoading) return <Spinner spinning />;
  if (error) return <Text color="red">Error loading config</Text>;
  if (!configJson) return <Text color="gray">Loading...</Text>;

  const canSaveToLocal = configItem.local_path !== "";
  const scopeChanged = targetScope !== configItem.scope;

  return (
    <Flex direction="column" gap="2" className={styles.configEditor}>
      {draftExpired && (
        <Callout.Root color="orange">
          <Callout.Icon>
            <InfoCircledIcon />
          </Callout.Icon>
          <Callout.Text>Draft expired</Callout.Text>
        </Callout.Root>
      )}
      {draft && <BuddyDraftPreview draft={draft} />}
      <Flex
        justify="between"
        align="center"
        wrap="wrap"
        gap="2"
        className={styles.editorHeader}
      >
        <Text size="2" weight="bold">
          {configId}
        </Text>
        <Flex gap="1" align="center">
          <SegmentedControl.Root
            size="1"
            value={view}
            onValueChange={(v) => setView(v as EditorView)}
          >
            <SegmentedControl.Item value="form">
              <MixerHorizontalIcon width={12} height={12} />
            </SegmentedControl.Item>
            <SegmentedControl.Item value="yaml">
              <CodeIcon width={12} height={12} />
            </SegmentedControl.Item>
          </SegmentedControl.Root>
          <Button
            size="1"
            onClick={() => void handleSave()}
            disabled={isSaving || !!yamlParseError}
          >
            {isSaving ? "..." : "Save"}
          </Button>
        </Flex>
      </Flex>
      {saveError && (
        <Text size="1" color="red">
          {saveError}
        </Text>
      )}
      {yamlParseError && (
        <Text size="1" color="red">
          YAML: {yamlParseError}
        </Text>
      )}
      <Flex align="center" gap="2" wrap="wrap" className={styles.scopeRow}>
        {canSaveToLocal ? (
          <SegmentedControl.Root
            size="1"
            value={targetScope}
            onValueChange={(v) => setTargetScope(v as "global" | "local")}
          >
            <SegmentedControl.Item value="global">
              <GlobeIcon width={10} height={10} />
            </SegmentedControl.Item>
            <SegmentedControl.Item value="local">
              <FileIcon width={10} height={10} />
            </SegmentedControl.Item>
          </SegmentedControl.Root>
        ) : (
          <Badge size="1" color="blue" variant="soft">
            <GlobeIcon width={10} height={10} />
          </Badge>
        )}
        {scopeChanged && (
          <Badge size="1" color="orange">
            → {targetScope}
          </Badge>
        )}
      </Flex>
      {view === "form" ? (
        <div className={styles.formContainer}>
          <FormEditor
            kind={kind}
            config={configJson}
            onPatch={handleFormPatch}
          />
        </div>
      ) : (
        <textarea
          className={styles.yamlEditor}
          value={yaml}
          onChange={(e) => handleYamlChange(e.target.value)}
          spellCheck={false}
        />
      )}
    </Flex>
  );
};

const FormEditor: React.FC<{
  kind: ConfigKind;
  config: Record<string, unknown>;
  onPatch: (patch: ConfigPatch) => void;
}> = ({ kind, config, onPatch }) => {
  switch (kind) {
    case "code_lens":
      return <CodeLensForm config={config} onPatch={onPatch} />;
    case "toolbox_commands":
      return <ToolboxCommandForm config={config} onPatch={onPatch} />;
    case "modes":
      return <ModeForm config={config} onPatch={onPatch} />;
    case "subagents":
      return <SubagentForm config={config} onPatch={onPatch} />;
  }
};

const CreateConfigDialog: React.FC<{
  kind: ConfigKind;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (id: string) => void;
  hasProjectRoot: boolean;
}> = ({ kind, open, onOpenChange, onCreated, hasProjectRoot }) => {
  const [id, setId] = useState("");
  const [scope, setScope] = useState<"global" | "local">(
    hasProjectRoot ? "local" : "global",
  );
  const [createConfig, { isLoading }] = useCreateConfigMutation();
  const [error, setError] = useState<string | null>(null);

  React.useEffect(() => {
    setScope(hasProjectRoot ? "local" : "global");
  }, [hasProjectRoot]);

  const handleCreate = useCallback(async () => {
    setError(null);
    const validationError = validateConfigId(id);
    if (validationError) {
      setError(validationError);
      return;
    }
    const defaultConfig = getDefaultConfig(kind, id);
    try {
      const result = await createConfig({
        kind,
        id,
        config: defaultConfig,
        scope,
      }).unwrap();
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
  }, [kind, id, scope, createConfig, onOpenChange, onCreated]);

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
          <Flex direction="column" gap="1">
            <Text size="1">Save to:</Text>
            {hasProjectRoot ? (
              <SegmentedControl.Root
                size="1"
                value={scope}
                onValueChange={(v) => setScope(v as "global" | "local")}
              >
                <SegmentedControl.Item value="global">
                  <Flex align="center" gap="1">
                    <GlobeIcon width={12} height={12} />
                    Global (~/.config/refact/)
                  </Flex>
                </SegmentedControl.Item>
                <SegmentedControl.Item value="local">
                  <Flex align="center" gap="1">
                    <FileIcon width={12} height={12} />
                    Project (.refact/)
                  </Flex>
                </SegmentedControl.Item>
              </SegmentedControl.Root>
            ) : (
              <Badge size="1" color="blue" variant="soft">
                <Flex align="center" gap="1">
                  <GlobeIcon width={10} height={10} />
                  Global only (no project open)
                </Flex>
              </Badge>
            )}
          </Flex>
          {error && (
            <Text size="2" color="red">
              {error}
            </Text>
          )}
        </Flex>
        <Flex gap="3" mt="4" justify="end">
          <Dialog.Close>
            <Button variant="soft" color="gray">
              Cancel
            </Button>
          </Dialog.Close>
          <Button onClick={() => void handleCreate()} disabled={isLoading}>
            {isLoading ? "Creating..." : "Create"}
          </Button>
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};

function getDefaultConfig(
  kind: ConfigKind,
  id: string,
): Record<string, unknown> {
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
  draftId,
}) => {
  const dispatch = useAppDispatch();
  const [activeKind, setActiveKind] = useState<ConfigKind>(initialKind);
  const [selectedConfigId, setSelectedConfigId] = useState<string | null>(
    initialConfigId ?? null,
  );
  const [createDialogOpen, setCreateDialogOpen] = useState(false);

  const { data: registry, isLoading, refetch } = useGetRegistryQuery(undefined);
  const [deleteConfig] = useDeleteConfigMutation();

  const getItemsForKind = (kind: ConfigKind): ConfigItem[] => {
    if (!registry) return [];
    switch (kind) {
      case "modes":
        return registry.modes;
      case "subagents":
        return registry.subagents;
      case "toolbox_commands":
        return registry.toolbox_commands;
      case "code_lens":
        return registry.code_lens;
    }
  };

  const getAllItems = (): ConfigItem[] => {
    if (!registry) return [];
    return [
      ...registry.modes,
      ...registry.subagents,
      ...registry.toolbox_commands,
      ...registry.code_lens,
    ];
  };

  const handleDelete = useCallback(
    async (id: string, scope: "global" | "local") => {
      if (!confirm(`Delete ${id} from ${scope}?`)) return;
      await deleteConfig({ kind: activeKind, id, scope });
      if (selectedConfigId === id) {
        setSelectedConfigId(null);
      }
      await refetch();
    },
    [activeKind, selectedConfigId, deleteConfig, refetch],
  );

  const handleTabChange = useCallback((value: string) => {
    setActiveKind(value as ConfigKind);
    setSelectedConfigId(null);
  }, []);

  if (isLoading) return <Spinner spinning />;

  return (
    <PageWrapper host={host} noPadding>
      {host === "vscode" && !tabbed ? (
        <Flex gap="2" pb="2">
          <Button variant="surface" onClick={backFromCustomization}>
            <ArrowLeftIcon width="16" height="16" />
            Back
          </Button>
        </Flex>
      ) : (
        <Button
          mr="auto"
          variant="outline"
          onClick={backFromCustomization}
          mb="2"
        >
          Back
        </Button>
      )}

      {registry?.errors && registry.errors.length > 0 && (
        <Card mb="3" style={{ backgroundColor: "var(--red-3)" }}>
          <Text size="2" color="red">
            {registry.errors.length} config error(s):{" "}
            {registry.errors.map((e) => e.error).join(", ")}
          </Text>
        </Card>
      )}

      <Tabs.Root value={activeKind} onValueChange={handleTabChange}>
        <Tabs.List size="1">
          {(Object.keys(KIND_LABELS) as ConfigKind[]).map((kind) => (
            <Tabs.Trigger key={kind} value={kind}>
              {KIND_LABELS[kind]} ({getItemsForKind(kind).length})
            </Tabs.Trigger>
          ))}
        </Tabs.List>

        <div className={styles.panelContainer}>
          {(() => {
            if (!selectedConfigId) {
              return (
                <ScrollArea scrollbars="vertical" className={styles.listPanel}>
                  {activeKind === "subagents" && (
                    <Flex justify="end" p="2">
                      <Button
                        size="1"
                        variant="outline"
                        onClick={() =>
                          dispatch(push({ name: "subagents marketplace" }))
                        }
                      >
                        <ExternalLinkIcon />
                        Browse Subagents Marketplace
                      </Button>
                    </Flex>
                  )}
                  <ConfigList
                    items={getItemsForKind(activeKind)}
                    selectedId={selectedConfigId}
                    onSelect={setSelectedConfigId}
                    onDelete={(id, scope) => void handleDelete(id, scope)}
                    onCreate={() => setCreateDialogOpen(true)}
                  />
                </ScrollArea>
              );
            }
            const selectedItem = getItemsForKind(activeKind).find(
              (i) => i.id === selectedConfigId,
            );
            if (!selectedItem) {
              return (
                <ScrollArea scrollbars="vertical" className={styles.listPanel}>
                  {activeKind === "subagents" && (
                    <Flex justify="end" p="2">
                      <Button
                        size="1"
                        variant="outline"
                        onClick={() =>
                          dispatch(push({ name: "subagents marketplace" }))
                        }
                      >
                        <ExternalLinkIcon />
                        Browse Subagents Marketplace
                      </Button>
                    </Flex>
                  )}
                  <ConfigList
                    items={getItemsForKind(activeKind)}
                    selectedId={selectedConfigId}
                    onSelect={setSelectedConfigId}
                    onDelete={(id, scope) => void handleDelete(id, scope)}
                    onCreate={() => setCreateDialogOpen(true)}
                  />
                </ScrollArea>
              );
            }
            return (
              <div className={styles.editorPanel}>
                <Button
                  variant="ghost"
                  size="1"
                  onClick={() => setSelectedConfigId(null)}
                  className={styles.backButton}
                >
                  <ArrowLeftIcon /> Back to list
                </Button>
                <ConfigEditor
                  kind={activeKind}
                  configId={selectedConfigId}
                  configItem={selectedItem}
                  onSaved={() => void refetch()}
                  draftId={draftId}
                />
              </div>
            );
          })()}
        </div>
      </Tabs.Root>

      <CreateConfigDialog
        kind={activeKind}
        open={createDialogOpen}
        onOpenChange={setCreateDialogOpen}
        onCreated={(id) => setSelectedConfigId(id)}
        hasProjectRoot={
          registry?.has_project_root ??
          getAllItems().some((i) => i.local_path !== "")
        }
      />
    </PageWrapper>
  );
};
