import React, { useState, useCallback, useRef, useEffect } from "react";
import {
  ArrowLeft,
  Plus,
  Trash2,
  Globe,
  File,
  Code,
  SlidersHorizontal,
  ExternalLink,
  Info,
} from "lucide-react";
import { skipToken } from "@reduxjs/toolkit/query";

import { PageWrapper } from "../../components/PageWrapper";
import {
  Badge,
  Button,
  Dialog,
  FieldText,
  Icon,
  IconButton,
  SegmentedControl,
  Spinner,
  Tabs,
  VirtualizedGrid,
} from "../../components/ui";
import { SettingsSection } from "../Settings/SettingsSection";
import {
  useGetRegistryQuery,
  useGetConfigQuery,
  useSaveConfigMutation,
  useCreateConfigMutation,
  useDeleteConfigMutation,
} from "../../services/refact/customization";
import type {
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
  validateConfigId,
  type ConfigPatch,
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
  embedded?: boolean;
};

const KIND_LABELS: Record<ConfigKind, string> = {
  modes: "Modes",
  subagents: "Subagents",
  toolbox_commands: "Toolbox",
  code_lens: "Code Lens",
};

const KIND_ORDER: ConfigKind[] = [
  "modes",
  "subagents",
  "toolbox_commands",
  "code_lens",
];

const CONFIG_ROW_GAP = 4;

const ConfigList: React.FC<{
  items: ConfigItem[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onDelete: (id: string, scope: "global" | "local") => void;
}> = ({ items, selectedId, onSelect, onDelete }) => {
  if (items.length === 0) {
    return <span className={styles.emptyText}>No configs found</span>;
  }
  return (
    <VirtualizedGrid
      items={items}
      columns={1}
      gap={CONFIG_ROW_GAP}
      getItemKey={(item) => item.id}
      aria-label="Configurations"
      renderItem={(item) => (
        <div
          role="button"
          tabIndex={0}
          className={`${styles.configRow} rf-pressable ${
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
          <div className={styles.rowInfo}>
            <span className={styles.rowTitle}>{item.title}</span>
            <span className={styles.rowId}>{item.id}</span>
          </div>
          <Badge
            className={styles.scopeBadge}
            tone={item.scope === "global" ? "accent" : "success"}
          >
            {item.scope === "global" ? "G" : "L"}
          </Badge>
          <IconButton
            aria-label={`Delete ${item.id}`}
            icon={Trash2}
            variant="ghost"
            size="sm"
            className={styles.deleteBtn}
            onClick={(e) => {
              e.stopPropagation();
              onDelete(item.id, item.scope);
            }}
          />
        </div>
      )}
    />
  );
};

type EditorView = "form" | "yaml";

const jsYamlPromise = import("js-yaml");

async function parseYamlConfig(
  yamlStr: string,
): Promise<Record<string, unknown>> {
  const jsYaml = await jsYamlPromise;
  const parsed = jsYaml.load(yamlStr);
  if (!isPlainObject(parsed)) {
    throw new Error("Config must be an object");
  }
  return sanitizeObject(parsed) as Record<string, unknown>;
}

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
          const parsed = await parseYamlConfig(draft.yaml_or_json);
          if (version !== syncVersionRef.current) return;
          setConfigJson(parsed);
          setYaml(draft.yaml_or_json);
          setYamlParseError(null);
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
        const parsed = await parseYamlConfig(yamlStr);
        if (version !== syncVersionRef.current) return;
        setConfigJson(parsed);
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
    if (yamlSyncTimeoutRef.current) {
      clearTimeout(yamlSyncTimeoutRef.current);
      yamlSyncTimeoutRef.current = null;
    }

    let configToSave = configJson;
    if (view === "yaml") {
      const version = ++syncVersionRef.current;
      try {
        configToSave = await parseYamlConfig(yaml);
        if (version !== syncVersionRef.current) return;
        setConfigJson(configToSave);
        setYamlParseError(null);
      } catch (e) {
        if (version !== syncVersionRef.current) return;
        setYamlParseError(e instanceof Error ? e.message : String(e));
        return;
      }
    }

    if (!configToSave) {
      setSaveError("No config to save");
      return;
    }
    try {
      const result = await saveConfig({
        kind,
        id: configId,
        config: configToSave,
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
  }, [
    configJson,
    view,
    kind,
    configId,
    saveConfig,
    onSaved,
    targetScope,
    draftId,
    yaml,
  ]);

  if (isLoading || draftLoading) return <Spinner />;
  if (error)
    return <span className={styles.errorText}>Error loading config</span>;
  if (!configJson) return <span className={styles.mutedText}>Loading...</span>;

  const canSaveToLocal = configItem.local_path !== "";
  const scopeChanged = targetScope !== configItem.scope;

  return (
    <div className={`${styles.configEditor} rf-enter-rise`}>
      {draftExpired && (
        <div className={styles.callout}>
          <Icon icon={Info} size="sm" tone="warning" />
          <span>Draft expired</span>
        </div>
      )}
      {draft && <BuddyDraftPreview draft={draft} />}
      <div className={styles.editorHeader}>
        <span className={styles.configTitle}>{configId}</span>
        <div className={styles.editorActions}>
          <SegmentedControl
            aria-label="Editor view"
            className={styles.editorToggle}
            size="sm"
            value={view}
            onValueChange={(v) => setView(v as EditorView)}
            options={[
              {
                value: "form",
                label: <Icon icon={SlidersHorizontal} size="sm" />,
                iconOnly: true,
                ariaLabel: "Form editor",
              },
              {
                value: "yaml",
                label: <Icon icon={Code} size="sm" />,
                iconOnly: true,
                ariaLabel: "YAML editor",
              },
            ]}
          />
          <Button
            size="sm"
            onClick={() => void handleSave()}
            disabled={isSaving}
          >
            {isSaving ? "..." : "Save"}
          </Button>
        </div>
      </div>
      {saveError && <span className={styles.errorText}>{saveError}</span>}
      {yamlParseError && (
        <span className={styles.errorText}>YAML: {yamlParseError}</span>
      )}
      <div className={styles.scopeRow}>
        {canSaveToLocal ? (
          <SegmentedControl
            aria-label="Save scope"
            className={styles.scopeToggle}
            size="sm"
            value={targetScope}
            onValueChange={(v) => setTargetScope(v as "global" | "local")}
            options={[
              {
                value: "global",
                label: <Icon icon={Globe} size="sm" />,
                iconOnly: true,
                ariaLabel: "Global scope",
              },
              {
                value: "local",
                label: <Icon icon={File} size="sm" />,
                iconOnly: true,
                ariaLabel: "Project scope",
              },
            ]}
          />
        ) : (
          <Badge tone="accent">
            <Icon icon={Globe} size="sm" />
          </Badge>
        )}
        {scopeChanged && <Badge tone="warning">→ {targetScope}</Badge>}
      </div>
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
    </div>
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
    <Dialog open={open} onOpenChange={onOpenChange}>
      <Dialog.Content maxWidth="calc(var(--rf-space-6) * 12)">
        <Dialog.Title>Create {KIND_LABELS[kind]}</Dialog.Title>
        <div className={styles.dialogBody}>
          <FieldText
            placeholder="Config ID (e.g., my_mode)"
            value={id}
            onChange={setId}
          />
          <div className={styles.scopeField}>
            <span className={styles.scopeLabel}>Save to:</span>
            {hasProjectRoot ? (
              <SegmentedControl
                aria-label="Config save scope"
                className={styles.dialogScopeToggle}
                value={scope}
                onValueChange={(v) => setScope(v as "global" | "local")}
                options={[
                  {
                    value: "global",
                    label: (
                      <span className={styles.scopeOption}>
                        <Icon icon={Globe} size="sm" />
                        Global (~/.config/refact/)
                      </span>
                    ),
                  },
                  {
                    value: "local",
                    label: (
                      <span className={styles.scopeOption}>
                        <Icon icon={File} size="sm" />
                        Project (.refact/)
                      </span>
                    ),
                  },
                ]}
              />
            ) : (
              <Badge tone="accent">
                <span className={styles.scopeOption}>
                  <Icon icon={Globe} size="sm" />
                  Global only (no project open)
                </span>
              </Badge>
            )}
          </div>
          {error && <span className={styles.errorText}>{error}</span>}
        </div>
        <div className={styles.dialogFooter}>
          <Button variant="soft" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={() => void handleCreate()} disabled={isLoading}>
            {isLoading ? "Creating..." : "Create"}
          </Button>
        </div>
      </Dialog.Content>
    </Dialog>
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
  embedded,
}) => {
  const dispatch = useAppDispatch();
  const [activeKind, setActiveKind] = useState<ConfigKind>(initialKind);
  const [selectedConfigId, setSelectedConfigId] = useState<string | null>(
    initialConfigId ?? null,
  );
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<{
    id: string;
    scope: "global" | "local";
  } | null>(null);

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

  if (isLoading) return <Spinner />;

  const activeIndex = KIND_ORDER.indexOf(activeKind);
  const activeItems = getItemsForKind(activeKind);
  const selectedItem = selectedConfigId
    ? activeItems.find((item) => item.id === selectedConfigId)
    : undefined;
  const showEditor = Boolean(selectedConfigId && selectedItem);

  const backButton = !embedded ? (
    host === "vscode" && !tabbed ? (
      <div className={styles.backRow}>
        <Button
          variant="soft"
          onClick={backFromCustomization}
          leftIcon={ArrowLeft}
        >
          Back
        </Button>
      </div>
    ) : (
      <div className={styles.backRow}>
        <Button
          variant="ghost"
          onClick={backFromCustomization}
          leftIcon={ArrowLeft}
        >
          Back
        </Button>
      </div>
    )
  ) : null;

  const actions = (
    <>
      {activeKind === "subagents" && (
        <Button
          size="sm"
          variant="ghost"
          rightIcon={ExternalLink}
          onClick={() => dispatch(push({ name: "subagents marketplace" }))}
        >
          Marketplace
        </Button>
      )}
      <Button
        size="sm"
        variant="soft"
        leftIcon={Plus}
        onClick={() => setCreateDialogOpen(true)}
      >
        New {KIND_LABELS[activeKind]}
      </Button>
    </>
  );

  const subNav = (
    <Tabs.List
      activeIndex={activeIndex}
      className={styles.kindTabs}
      itemCount={KIND_ORDER.length}
    >
      {KIND_ORDER.map((kind) => (
        <Tabs.Trigger key={kind} value={kind}>
          <span className={styles.tabTriggerContent}>
            <span className={styles.tabText}>{KIND_LABELS[kind]}</span>
            <Badge className={styles.tabCount} tone="muted">
              {getItemsForKind(kind).length}
            </Badge>
          </span>
        </Tabs.Trigger>
      ))}
    </Tabs.List>
  );

  const inner = (
    <div className={`${styles.pageShell} rf-enter`}>
      {backButton}

      {registry?.errors && registry.errors.length > 0 && (
        <div className={styles.errorBanner}>
          <span className={styles.errorText}>
            {registry.errors.length} config error(s):{" "}
            {registry.errors.map((e) => e.error).join(", ")}
          </span>
        </div>
      )}

      <Tabs value={activeKind} onValueChange={handleTabChange}>
        <SettingsSection
          title="Customization"
          description="Tune modes, subagents, toolbox commands, and code lens actions."
          actions={actions}
          subNav={subNav}
          width={showEditor ? "wide" : "default"}
        >
          {selectedConfigId && selectedItem ? (
            <div className={styles.editorPanel}>
              <Button
                variant="ghost"
                size="sm"
                leftIcon={ArrowLeft}
                onClick={() => setSelectedConfigId(null)}
                className={styles.backToListBtn}
              >
                Back to list
              </Button>
              <ConfigEditor
                kind={activeKind}
                configId={selectedConfigId}
                configItem={selectedItem}
                onSaved={() => void refetch()}
                draftId={draftId}
              />
            </div>
          ) : (
            <div className={styles.listPanel}>
              <ConfigList
                items={activeItems}
                selectedId={selectedConfigId}
                onSelect={setSelectedConfigId}
                onDelete={(id, scope) => setDeleteTarget({ id, scope })}
              />
            </div>
          )}
        </SettingsSection>
      </Tabs>

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
      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
      >
        <Dialog.Content maxWidth="calc(var(--rf-space-6) * 12)">
          <Dialog.Title>Delete configuration?</Dialog.Title>
          <Dialog.Description>
            {deleteTarget
              ? `Delete ${deleteTarget.id} from ${deleteTarget.scope}?`
              : "Delete this configuration?"}
          </Dialog.Description>
          <div className={styles.dialogFooter}>
            <Button variant="soft" onClick={() => setDeleteTarget(null)}>
              Cancel
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                if (!deleteTarget) return;
                const { id, scope } = deleteTarget;
                setDeleteTarget(null);
                void handleDelete(id, scope);
              }}
            >
              Delete
            </Button>
          </div>
        </Dialog.Content>
      </Dialog>
    </div>
  );

  if (embedded) return inner;
  return (
    <PageWrapper host={host} noPadding>
      {inner}
    </PageWrapper>
  );
};
