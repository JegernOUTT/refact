import React, { useState, useCallback, useEffect } from "react";
import { Code, Info, Plus, SlidersHorizontal, Trash2 } from "lucide-react";
import {
  Badge,
  Button,
  EmptyState,
  FieldError,
  FieldSelect,
  FieldStack,
  FieldText,
  Icon,
  IconButton,
  SegmentedControl,
} from "../../../components/ui";
import {
  useGetHooksQuery,
  useSaveHooksMutation,
  type HookEntry,
} from "../../../services/refact/extensions";
import { Spinner } from "../../../components/Spinner";
import styles from "./HooksEditor.module.css";
import featureStyles from "../../featureUi.module.css";

const HOOK_EVENTS = [
  "PreToolUse",
  "PostToolUse",
  "UserPromptSubmit",
  "SessionStart",
  "SessionEnd",
  "Stop",
  "PreCompact",
] as const;

type HookEvent = (typeof HOOK_EVENTS)[number];

const EVENTS_WITH_MATCHER: HookEvent[] = ["PreToolUse", "PostToolUse"];

const hookEventOptions = HOOK_EVENTS.map((event) => ({
  value: event,
  label: event,
}));

type HookRowProps = {
  hook: HookEntry;
  index: number;
  onUpdate: (index: number, updated: HookEntry) => void;
  onDelete: (index: number) => void;
};

const HookRow: React.FC<HookRowProps> = ({
  hook,
  index,
  onUpdate,
  onDelete,
}) => {
  const showMatcher = EVENTS_WITH_MATCHER.includes(hook.event as HookEvent);

  return (
    <div className={`${styles.hookRow} rf-enter`}>
      <div className={styles.hookHeader}>
        <Badge tone="accent">{hook.event}</Badge>
        {hook.matcher && (
          <span className={styles.matcher}>matcher: {hook.matcher}</span>
        )}
        <IconButton
          variant="danger"
          size="sm"
          icon={Trash2}
          aria-label={`Delete hook ${index}`}
          onClick={() => onDelete(index)}
        />
      </div>

      <FieldStack
        label="Event"
        control={
          <FieldSelect
            value={hook.event}
            onChange={(event) => onUpdate(index, { ...hook, event })}
            options={hookEventOptions}
          />
        }
      />

      {showMatcher && (
        <FieldStack
          label="Matcher (optional regex)"
          control={
            <FieldText
              value={hook.matcher ?? ""}
              onChange={(matcher) =>
                onUpdate(index, {
                  ...hook,
                  matcher: matcher || undefined,
                })
              }
              placeholder="Tool name regex, e.g., shell.*"
            />
          }
        />
      )}

      <FieldStack
        label="Command"
        control={
          <textarea
            className={styles.commandTextarea}
            value={hook.command}
            onChange={(event) =>
              onUpdate(index, { ...hook, command: event.target.value })
            }
            placeholder="Shell command to run..."
            spellCheck={false}
          />
        }
      />

      <FieldStack
        label="Timeout (seconds, optional)"
        control={
          <FieldText
            type="number"
            value={hook.timeout !== undefined ? String(hook.timeout) : ""}
            onChange={(timeout) =>
              onUpdate(index, {
                ...hook,
                timeout: timeout ? parseInt(timeout, 10) : undefined,
              })
            }
            placeholder="30"
          />
        }
      />
    </div>
  );
};

type EditorView = "form" | "raw";

type HooksEditorProps = {
  scope?: "global" | "local";
};

function hooksEqual(left: HookEntry[], right: HookEntry[]): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

export const HooksEditor: React.FC<HooksEditorProps> = ({ scope }) => {
  const [hooksScope, setHooksScope] = useState<"global" | "local">(
    scope ?? "global",
  );
  const {
    currentData: data,
    isLoading,
    isFetching,
    error,
  } = useGetHooksQuery({ scope: hooksScope });
  const [saveHooks, { isLoading: isSaving }] = useSaveHooksMutation();
  const [view, setView] = useState<EditorView>("form");
  const [hooks, setHooks] = useState<HookEntry[]>([]);
  const [rawYaml, setRawYaml] = useState("");
  const [saveError, setSaveError] = useState<string | null>(null);
  const [loadedScope, setLoadedScope] = useState<"global" | "local" | null>(
    null,
  );
  const [loadedHooks, setLoadedHooks] = useState<HookEntry[]>([]);
  const [loadedRawYaml, setLoadedRawYaml] = useState("");

  useEffect(() => {
    if (data) {
      setHooks(data.hooks);
      setRawYaml(data.raw_yaml);
      setLoadedHooks(data.hooks);
      setLoadedRawYaml(data.raw_yaml);
      setLoadedScope(hooksScope);
      setSaveError(null);
    }
  }, [data, hooksScope]);

  const hasUnsavedEdits =
    loadedScope === hooksScope &&
    (!hooksEqual(hooks, loadedHooks) || rawYaml !== loadedRawYaml);

  const resetLoadedState = useCallback(() => {
    setHooks([]);
    setRawYaml("");
    setLoadedHooks([]);
    setLoadedRawYaml("");
    setLoadedScope(null);
    setSaveError(null);
  }, []);

  const handleScopeChange = useCallback(
    (value: string) => {
      const nextScope = value as "global" | "local";
      if (nextScope === hooksScope) return;
      if (
        hasUnsavedEdits &&
        !window.confirm("Discard unsaved hook changes before switching scope?")
      ) {
        return;
      }
      resetLoadedState();
      setHooksScope(nextScope);
    },
    [hasUnsavedEdits, hooksScope, resetLoadedState],
  );

  const handleUpdate = useCallback((index: number, updated: HookEntry) => {
    setHooks((prev) => prev.map((h, i) => (i === index ? updated : h)));
  }, []);

  const handleDelete = useCallback((index: number) => {
    setHooks((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const handleAdd = useCallback(() => {
    setHooks((prev) => [
      ...prev,
      {
        event: "PreToolUse",
        command: "",
        matcher: undefined,
        timeout: undefined,
      },
    ]);
  }, []);

  const handleSave = useCallback(async () => {
    setSaveError(null);
    if (loadedScope !== hooksScope) return;
    try {
      if (view === "raw") {
        await saveHooks({
          scope: hooksScope,
          body: { raw_yaml: rawYaml },
        }).unwrap();
      } else {
        await saveHooks({ scope: hooksScope, body: { hooks } }).unwrap();
      }
      setLoadedHooks(hooks);
      setLoadedRawYaml(rawYaml);
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    }
  }, [view, hooksScope, loadedScope, rawYaml, hooks, saveHooks]);

  if (isLoading || (isFetching && loadedScope !== hooksScope)) {
    return <Spinner spinning />;
  }
  if (error) {
    return (
      <div
        className={`${featureStyles.callout} ${featureStyles.calloutDanger}`}
      >
        <Icon icon={Info} size="sm" tone="danger" />
        Failed to load hooks
      </div>
    );
  }

  return (
    <div className={`${styles.editor} rf-enter`}>
      <div className={styles.header}>
        <div className={styles.headerTitle}>
          <h2 className={styles.title}>Hooks</h2>
          <SegmentedControl
            size="sm"
            value={hooksScope}
            onValueChange={handleScopeChange}
            options={[
              { value: "global", label: "Global" },
              { value: "local", label: "Project" },
            ]}
          />
        </div>
        <div className={styles.headerActions}>
          <SegmentedControl
            size="sm"
            value={view}
            onValueChange={(value) => setView(value as EditorView)}
            options={[
              {
                value: "form",
                label: <Icon icon={SlidersHorizontal} size="sm" />,
              },
              { value: "raw", label: <Icon icon={Code} size="sm" /> },
            ]}
          />
          <Button
            variant="primary"
            size="sm"
            onClick={() => void handleSave()}
            disabled={isSaving || loadedScope !== hooksScope}
            loading={isSaving}
          >
            Save
          </Button>
        </div>
      </div>

      {saveError && <FieldError>{saveError}</FieldError>}

      {view === "form" ? (
        <div className={`${styles.formContent} rf-stagger`}>
          {hooks.map((hook, index) => (
            <HookRow
              key={index}
              hook={hook}
              index={index}
              onUpdate={handleUpdate}
              onDelete={handleDelete}
            />
          ))}
          {hooks.length === 0 && (
            <EmptyState
              title="No hooks configured"
              description="Add a hook to run commands on lifecycle events."
            />
          )}
          <Button variant="soft" size="sm" onClick={handleAdd} leftIcon={Plus}>
            Add Hook
          </Button>
        </div>
      ) : (
        <textarea
          className={styles.rawTextarea}
          value={rawYaml}
          onChange={(e) => setRawYaml(e.target.value)}
          spellCheck={false}
        />
      )}
    </div>
  );
};
