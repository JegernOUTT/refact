import React, { useState, useCallback, useEffect } from "react";
import {
  Flex,
  Button,
  Text,
  TextField,
  Select,
  SegmentedControl,
  Badge,
  IconButton,
  Callout,
} from "@radix-ui/themes";
import {
  PlusIcon,
  TrashIcon,
  CodeIcon,
  MixerHorizontalIcon,
  InfoCircledIcon,
} from "@radix-ui/react-icons";
import {
  useGetHooksQuery,
  useSaveHooksMutation,
  type HookEntry,
} from "../../../services/refact/extensions";
import { Spinner } from "../../../components/Spinner";
import styles from "./HooksEditor.module.css";

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
    <Flex direction="column" gap="2" className={styles.hookRow}>
      <Flex gap="2" align="center" wrap="wrap">
        <Badge size="1" variant="soft">
          {hook.event}
        </Badge>
        {hook.matcher && (
          <Text size="1" color="gray">
            matcher: {hook.matcher}
          </Text>
        )}
        <IconButton
          size="1"
          variant="ghost"
          color="red"
          aria-label={`Delete hook ${index}`}
          onClick={() => onDelete(index)}
          style={{ marginLeft: "auto" }}
        >
          <TrashIcon />
        </IconButton>
      </Flex>

      <Flex direction="column" gap="1">
        <Text size="1" weight="medium">
          Event
        </Text>
        <Select.Root
          size="1"
          value={hook.event}
          onValueChange={(v) => onUpdate(index, { ...hook, event: v })}
        >
          <Select.Trigger style={{ width: "100%" }} />
          <Select.Content>
            {HOOK_EVENTS.map((e) => (
              <Select.Item key={e} value={e}>
                {e}
              </Select.Item>
            ))}
          </Select.Content>
        </Select.Root>
      </Flex>

      {showMatcher && (
        <Flex direction="column" gap="1">
          <Text size="1" weight="medium">
            Matcher (optional regex)
          </Text>
          <TextField.Root
            size="1"
            value={hook.matcher ?? ""}
            onChange={(e) =>
              onUpdate(index, {
                ...hook,
                matcher: e.target.value || undefined,
              })
            }
            placeholder="Tool name regex, e.g., shell.*"
          />
        </Flex>
      )}

      <Flex direction="column" gap="1">
        <Text size="1" weight="medium">
          Command
        </Text>
        <textarea
          className={styles.commandTextarea}
          value={hook.command}
          onChange={(e) =>
            onUpdate(index, { ...hook, command: e.target.value })
          }
          placeholder="Shell command to run..."
          spellCheck={false}
        />
      </Flex>

      <Flex direction="column" gap="1">
        <Text size="1" weight="medium">
          Timeout (seconds, optional)
        </Text>
        <TextField.Root
          size="1"
          type="number"
          value={hook.timeout !== undefined ? String(hook.timeout) : ""}
          onChange={(e) =>
            onUpdate(index, {
              ...hook,
              timeout: e.target.value
                ? parseInt(e.target.value, 10)
                : undefined,
            })
          }
          placeholder="30"
        />
      </Flex>
    </Flex>
  );
};

type EditorView = "form" | "raw";

type HooksEditorProps = {
  scope?: "global" | "local";
};

export const HooksEditor: React.FC<HooksEditorProps> = ({ scope }) => {
  const { data, isLoading, error } = useGetHooksQuery({ scope });
  const [saveHooks, { isLoading: isSaving }] = useSaveHooksMutation();
  const [view, setView] = useState<EditorView>("form");
  const [hooks, setHooks] = useState<HookEntry[]>([]);
  const [rawYaml, setRawYaml] = useState("");
  const [saveError, setSaveError] = useState<string | null>(null);
  const [hooksScope, setHooksScope] = useState<"global" | "local">(
    scope ?? "global",
  );

  useEffect(() => {
    if (data) {
      setHooks(data.hooks);
      setRawYaml(data.raw_yaml);
    }
  }, [data]);

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
    try {
      if (view === "raw") {
        await saveHooks({
          scope: hooksScope,
          body: { raw_yaml: rawYaml },
        }).unwrap();
      } else {
        await saveHooks({ scope: hooksScope, body: { hooks } }).unwrap();
      }
    } catch (e) {
      setSaveError(e instanceof Error ? e.message : String(e));
    }
  }, [view, hooksScope, rawYaml, hooks, saveHooks]);

  if (isLoading) return <Spinner spinning />;
  if (error) {
    return (
      <Callout.Root color="red">
        <Callout.Icon>
          <InfoCircledIcon />
        </Callout.Icon>
        <Callout.Text>Failed to load hooks</Callout.Text>
      </Callout.Root>
    );
  }

  return (
    <Flex direction="column" gap="2" className={styles.editor}>
      <Flex justify="between" align="center" gap="2" wrap="wrap">
        <Flex gap="2" align="center">
          <Text size="2" weight="bold">
            Hooks
          </Text>
          <SegmentedControl.Root
            size="1"
            value={hooksScope}
            onValueChange={(v) => setHooksScope(v as "global" | "local")}
          >
            <SegmentedControl.Item value="global">Global</SegmentedControl.Item>
            <SegmentedControl.Item value="local">Project</SegmentedControl.Item>
          </SegmentedControl.Root>
        </Flex>
        <Flex gap="1" align="center">
          <SegmentedControl.Root
            size="1"
            value={view}
            onValueChange={(v) => setView(v as EditorView)}
          >
            <SegmentedControl.Item value="form">
              <MixerHorizontalIcon width={12} height={12} />
            </SegmentedControl.Item>
            <SegmentedControl.Item value="raw">
              <CodeIcon width={12} height={12} />
            </SegmentedControl.Item>
          </SegmentedControl.Root>
          <Button
            size="1"
            onClick={() => void handleSave()}
            disabled={isSaving}
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

      {view === "form" ? (
        <Flex direction="column" gap="3" className={styles.formContent}>
          {hooks.map((hook, i) => (
            <HookRow
              key={i}
              hook={hook}
              index={i}
              onUpdate={handleUpdate}
              onDelete={handleDelete}
            />
          ))}
          {hooks.length === 0 && (
            <Text size="1" color="gray">
              No hooks configured.
            </Text>
          )}
          <Button variant="soft" size="1" onClick={handleAdd}>
            <PlusIcon /> Add Hook
          </Button>
        </Flex>
      ) : (
        <textarea
          className={styles.rawTextarea}
          value={rawYaml}
          onChange={(e) => setRawYaml(e.target.value)}
          spellCheck={false}
        />
      )}
    </Flex>
  );
};
