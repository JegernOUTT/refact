import { useEffect, useState } from "react";
import { Callout } from "@radix-ui/themes";
import classNames from "classnames";

import {
  Badge,
  Button,
  Card,
  DataTable,
  ErrorState,
  FieldSelect,
  FieldSwitch,
  FieldText,
  Flex,
  LoadingState,
  SaveStatus,
  SettingItem,
  SettingsListEditor,
  Text,
  type SaveStatusState,
} from "../../components/ui";
import {
  type ShellApprovalMode,
  type ShellAuditEntry,
  type ShellGateDecision,
  type ShellLlmAuthority,
  type ShellLlmOnFailure,
  type ShellPolicy,
  type ShellRiskLevel,
  useGetShellAuditQuery,
  useGetShellPolicyQuery,
  useTestShellCommandMutation,
  useUpdateShellPolicyMutation,
} from "../../services/refact/shellPolicy";
import { SettingsGroup, SettingsSection } from "../Settings/SettingsSection";
import styles from "./ShellSettingsSection.module.css";

const MODE_OPTIONS = [
  { value: "strict", label: "Strict — ask for anything not allow-listed" },
  { value: "balanced", label: "Balanced (recommended)" },
  {
    value: "permissive",
    label: "Permissive — ask only for High and Critical",
  },
  { value: "yolo", label: "YOLO — skip routine confirmation" },
];

const MODE_DESCRIPTIONS: Record<ShellApprovalMode, string> = {
  strict: "Ask before every command that is not explicitly allow-listed.",
  balanced: "Ask before running commands classified Medium risk or higher.",
  permissive: "Ask only for High and Critical risk commands.",
  yolo: "Never ask, except when a deny rule matches or the model explicitly requests confirmation.",
};

const RISK_OPTIONS = [
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "critical", label: "Critical" },
];

const AUDIT_LIMIT = 50;
const SECOND_MS = 1_000;
const MINUTE_MS = 60 * SECOND_MS;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;
const COMMAND_PREVIEW_LENGTH = 80;

function decisionTone(decision: ShellGateDecision) {
  if (decision === "pass") return "success" as const;
  if (decision === "confirmation") return "warning" as const;
  return "danger" as const;
}

function formatRelativeTime(timestampMs: number): string {
  const elapsed = Math.max(0, Date.now() - timestampMs);
  if (elapsed < MINUTE_MS) return "just now";
  if (elapsed < HOUR_MS)
    return `${String(Math.floor(elapsed / MINUTE_MS))}m ago`;
  if (elapsed < DAY_MS) return `${String(Math.floor(elapsed / HOUR_MS))}h ago`;
  return `${String(Math.floor(elapsed / DAY_MS))}d ago`;
}

function commandPreview(command: string): string {
  return command.length > COMMAND_PREVIEW_LENGTH
    ? `${command.slice(0, COMMAND_PREVIEW_LENGTH)}…`
    : command;
}

function firstCommandSegment(command: string): string | undefined {
  const match = command.trim().match(/^(?:"([^"]+)"|'([^']+)'|(\S+))/);
  return match?.[1] ?? match?.[2] ?? match?.[3];
}

function errorMessage(error: unknown): string {
  if (typeof error === "object" && error !== null) {
    if ("error" in error && typeof error.error === "string") {
      return error.error;
    }
    if (
      "data" in error &&
      typeof error.data === "object" &&
      error.data !== null
    ) {
      if ("message" in error.data && typeof error.data.message === "string") {
        return error.data.message;
      }
      if ("detail" in error.data && typeof error.data.detail === "string") {
        return error.data.detail;
      }
    }
  }
  return "The command could not be evaluated.";
}

type ShellRuleTone = "deny" | "ask" | "allow";

const RULE_PLACEHOLDER: Record<ShellRuleTone, string> = {
  deny: "rm -rf *",
  ask: "git push*",
  allow: "ls",
};

const RULE_BADGE_TONE: Record<ShellRuleTone, "danger" | "warning" | "success"> =
  {
    deny: "danger",
    ask: "warning",
    allow: "success",
  };

interface ShellRuleBlockProps {
  title: string;
  hint: string;
  tone: ShellRuleTone;
  rules: string[];
  savedRules: string[];
  disabled: boolean;
  onChange: (rules: string[]) => void;
}

function ShellRuleBlock({
  disabled,
  hint,
  onChange,
  rules,
  savedRules,
  title,
  tone,
}: ShellRuleBlockProps) {
  const dirty =
    rules.length !== savedRules.length ||
    rules.some((rule, index) => rule !== savedRules[index]);

  const items = rules.map((value, index) => ({
    id: String(index),
    value,
  }));

  return (
    <SettingItem
      layout="stack"
      title={
        <span className={styles.ruleTitle}>
          <span
            aria-hidden="true"
            className={classNames(styles.ruleMarker, styles[tone])}
          />
          {title}
          <Badge tone={RULE_BADGE_TONE[tone]}>{rules.length}</Badge>
        </span>
      }
      description={hint}
      control={
        <Flex direction="column" gap="2" align="stretch">
          <SettingsListEditor
            addLabel="Add rule"
            emptyLabel="No rules yet."
            items={items}
            itemAriaLabel={(_item, index) =>
              `Remove rule ${String(index + 1)} from ${title}`
            }
            monospace
            placeholder={RULE_PLACEHOLDER[tone]}
            disabled={disabled}
            onAdd={() => onChange([...rules, ""])}
            onChange={(id, value) =>
              onChange(
                rules.map((rule, index) =>
                  String(index) === id ? value : rule,
                ),
              )
            }
            onRemove={(id) =>
              onChange(rules.filter((_rule, index) => String(index) !== id))
            }
          />
          {dirty ? (
            <Flex justify="end">
              <Button
                disabled={disabled}
                size="sm"
                variant="ghost"
                onClick={() => onChange([...savedRules])}
              >
                Revert
              </Button>
            </Flex>
          ) : null}
        </Flex>
      }
    />
  );
}

export function ShellSettingsSection() {
  const policyQuery = useGetShellPolicyQuery(undefined);
  const auditQuery = useGetShellAuditQuery({ limit: AUDIT_LIMIT });
  const [updatePolicy, updateState] = useUpdateShellPolicyMutation();
  const [testCommand, testState] = useTestShellCommandMutation();
  const [draft, setDraft] = useState<ShellPolicy | undefined>();
  const [command, setCommand] = useState("");

  useEffect(() => {
    if (policyQuery.data) setDraft(policyQuery.data);
  }, [policyQuery.data]);

  const saveStatus: SaveStatusState = updateState.isLoading
    ? "saving"
    : updateState.isSuccess
      ? "saved"
      : updateState.isError
        ? "error"
        : "idle";

  const sectionDescription =
    "Control when shell commands need your approval. Deny rules are always enforced, in every mode.";

  const evaluateCommand = () => {
    if (command.trim() && !testState.isLoading) {
      void testCommand({ command });
    }
  };

  if (policyQuery.isLoading) {
    return (
      <SettingsSection title="Shell" description={sectionDescription}>
        <LoadingState label="Loading shell policy" variant="full" />
      </SettingsSection>
    );
  }

  if (policyQuery.isError || !policyQuery.data) {
    return (
      <SettingsSection title="Shell" description={sectionDescription}>
        <ErrorState
          title="Failed to load shell policy"
          description="The shell policy endpoint could not be reached."
          retry={
            <Button variant="soft" onClick={() => void policyQuery.refetch()}>
              Retry
            </Button>
          }
          variant="full"
        />
      </SettingsSection>
    );
  }

  if (!draft) {
    return (
      <SettingsSection title="Shell" description={sectionDescription}>
        <LoadingState label="Loading shell policy" variant="full" />
      </SettingsSection>
    );
  }

  const serverPolicy = policyQuery.data;
  const saving = updateState.isLoading;

  return (
    <SettingsSection
      title="Shell"
      description={sectionDescription}
      width="wide"
    >
      <SettingsGroup title="Approval">
        <SettingItem
          layout="stack"
          title="Approval mode"
          description={MODE_DESCRIPTIONS[draft.mode]}
          control={
            <Flex direction="column" gap="2" align="stretch">
              <FieldSelect
                aria-label="Approval mode"
                disabled={saving}
                options={MODE_OPTIONS}
                value={draft.mode}
                onChange={(value) =>
                  setDraft({ ...draft, mode: value as ShellApprovalMode })
                }
              />
              {draft.mode === "yolo" ? (
                <Card padding="sm" variant="surface-2" role="alert">
                  <Text as="p" color="orange">
                    YOLO skips routine confirmation, but deny rules still block
                    and explicit model requests still ask. Use this only in a
                    sandbox or throwaway container.
                  </Text>
                </Card>
              ) : null}
            </Flex>
          }
        />
        <SettingItem
          title="Trust the model's confirmation request"
          description="Honour needs_confirmation: true when a tool call sets it. Escalate-only — the model can raise the gate, never lower it."
          control={
            <FieldSwitch
              aria-label="Trust the model's confirmation request"
              checked={draft.trust_caller_confirmation}
              disabled={saving}
              onChange={(checked) =>
                setDraft({ ...draft, trust_caller_confirmation: checked })
              }
            />
          }
        />
      </SettingsGroup>

      <SettingsGroup
        title="Rules"
        description="A rule with no spaces matches the program name (rm, mkfs*); a rule with spaces matches the whole command (git push*). Prefixes exec:, argv:, re:, and raw: force a specific target; raw: is the old whole-line matching."
      >
        <ShellRuleBlock
          tone="deny"
          title="Never run (deny)"
          hint="Always blocked, in every mode."
          rules={draft.deny}
          savedRules={serverPolicy.deny}
          disabled={saving}
          onChange={(deny) => setDraft({ ...draft, deny })}
        />
        <ShellRuleBlock
          tone="ask"
          title="Always ask"
          hint="Always requires your approval before running."
          rules={draft.ask}
          savedRules={serverPolicy.ask}
          disabled={saving}
          onChange={(ask) => setDraft({ ...draft, ask })}
        />
        <ShellRuleBlock
          tone="allow"
          title="Never ask (allow)"
          hint="Runs without confirmation. Deny rules still win."
          rules={draft.allow}
          savedRules={serverPolicy.allow}
          disabled={saving}
          onChange={(allow) => setDraft({ ...draft, allow })}
        />
      </SettingsGroup>

      <SettingsGroup title="Test a command">
        <SettingItem
          layout="stack"
          title="Command"
          control={
            <Flex direction="column" gap="2" align="stretch">
              <div className={styles.testRow}>
                <FieldText
                  aria-label="Command to evaluate"
                  className={styles.testInput}
                  disabled={testState.isLoading}
                  placeholder="git push origin main"
                  spellCheck={false}
                  value={command}
                  onChange={setCommand}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") evaluateCommand();
                  }}
                />
                <Button
                  disabled={!command.trim()}
                  loading={testState.isLoading}
                  onClick={evaluateCommand}
                >
                  {testState.isLoading ? "Evaluating…" : "Evaluate"}
                </Button>
              </div>
              {testState.data ? (
                <Callout.Root
                  color={
                    testState.data.decision === "pass"
                      ? "green"
                      : testState.data.decision === "confirmation"
                        ? "amber"
                        : "red"
                  }
                >
                  <Callout.Text>
                    <Flex direction="column" gap="1">
                      <strong>{testState.data.decision}</strong>
                      <span>{testState.data.reason}</span>
                      <span>
                        Rule: <code>{testState.data.rule}</code>
                      </span>
                      {testState.data.risk_level ? (
                        <span>Risk level: {testState.data.risk_level}</span>
                      ) : null}
                      <span>
                        Segments:{" "}
                        {testState.data.segments.map((segment, index) => (
                          <code key={`${segment}-${String(index)}`}>
                            {segment}{" "}
                          </code>
                        ))}
                      </span>
                    </Flex>
                  </Callout.Text>
                </Callout.Root>
              ) : null}
              {testState.isError ? (
                <Callout.Root color="red">
                  <Callout.Text>{errorMessage(testState.error)}</Callout.Text>
                </Callout.Root>
              ) : null}
            </Flex>
          }
        />
      </SettingsGroup>

      <SettingsGroup title="Risk catalogue">
        {draft.catalogue.map((entry, index) => (
          <SettingItem
            key={entry.id}
            title={`${entry.id} — ${entry.exec}`}
            description={entry.reason}
            control={
              <div className={styles.riskControl}>
                <span className={styles.riskSelect}>
                  <FieldSelect
                    aria-label={`Risk level for ${entry.id}`}
                    disabled={saving}
                    options={RISK_OPTIONS}
                    value={entry.level}
                    onChange={(value) =>
                      setDraft({
                        ...draft,
                        catalogue: draft.catalogue.map((item, itemIndex) =>
                          itemIndex === index
                            ? { ...item, level: value as ShellRiskLevel }
                            : item,
                        ),
                      })
                    }
                  />
                </span>
                <FieldSwitch
                  aria-label={`Enable ${entry.id}`}
                  checked={entry.enabled}
                  disabled={saving}
                  onChange={(checked) =>
                    setDraft({
                      ...draft,
                      catalogue: draft.catalogue.map((item, itemIndex) =>
                        itemIndex === index
                          ? { ...item, enabled: checked }
                          : item,
                      ),
                    })
                  }
                />
              </div>
            }
          />
        ))}
        <Flex align="start">
          <Button
            disabled={saving}
            size="sm"
            variant="soft"
            onClick={() =>
              setDraft({
                ...draft,
                catalogue: serverPolicy.catalogue.map((entry) => ({
                  ...entry,
                })),
              })
            }
          >
            Revert all
          </Button>
        </Flex>
      </SettingsGroup>

      <SettingsGroup title="AI validation">
        <SettingItem
          title="Check commands with a model before running"
          description="Adds model latency and token cost before shell commands run."
          control={
            <FieldSwitch
              aria-label="Check commands with a model before running"
              checked={draft.llm_validation.enabled}
              disabled={saving}
              onChange={(checked) =>
                setDraft({
                  ...draft,
                  llm_validation: {
                    ...draft.llm_validation,
                    enabled: checked,
                  },
                })
              }
            />
          }
        />
        {draft.llm_validation.enabled ? (
          <>
            <SettingItem
              title="Model"
              control={
                <FieldText
                  aria-label="Validation model"
                  disabled={saving}
                  placeholder="chat_light_model"
                  value={draft.llm_validation.model}
                  onChange={(model) =>
                    setDraft({
                      ...draft,
                      llm_validation: { ...draft.llm_validation, model },
                    })
                  }
                />
              }
            />
            <SettingItem
              title="Authority"
              control={
                <FieldSelect
                  aria-label="Validation authority"
                  disabled={saving}
                  options={[
                    {
                      value: "ask_only",
                      label: "Can only ask for approval (safe)",
                    },
                    {
                      value: "ask_and_allow",
                      label: "Can ask and can approve",
                    },
                  ]}
                  value={draft.llm_validation.authority}
                  onChange={(authority) =>
                    setDraft({
                      ...draft,
                      llm_validation: {
                        ...draft.llm_validation,
                        authority: authority as ShellLlmAuthority,
                      },
                    })
                  }
                />
              }
            />
            <SettingItem
              title="Timeout seconds"
              control={
                <FieldText
                  aria-label="Validation timeout seconds"
                  disabled={saving}
                  inputMode="numeric"
                  type="number"
                  value={String(draft.llm_validation.timeout_secs)}
                  onChange={(value) =>
                    setDraft({
                      ...draft,
                      llm_validation: {
                        ...draft.llm_validation,
                        timeout_secs: Number(value),
                      },
                    })
                  }
                />
              }
            />
            <SettingItem
              title="If validation fails"
              control={
                <FieldSelect
                  aria-label="If validation fails"
                  disabled={saving}
                  options={[
                    { value: "pass", label: "Run the command (fail open)" },
                    { value: "ask", label: "Ask me (fail closed)" },
                  ]}
                  value={draft.llm_validation.on_failure}
                  onChange={(onFailure) =>
                    setDraft({
                      ...draft,
                      llm_validation: {
                        ...draft.llm_validation,
                        on_failure: onFailure as ShellLlmOnFailure,
                      },
                    })
                  }
                />
              }
            />
            <SettingItem
              title="Cache identical commands"
              control={
                <FieldSwitch
                  aria-label="Cache identical commands"
                  checked={draft.llm_validation.cache_per_chat}
                  disabled={saving}
                  onChange={(checked) =>
                    setDraft({
                      ...draft,
                      llm_validation: {
                        ...draft.llm_validation,
                        cache_per_chat: checked,
                      },
                    })
                  }
                />
              }
            />
          </>
        ) : null}
      </SettingsGroup>

      <SettingsGroup title="Execution defaults">
        <SettingItem
          title="Foreground timeout seconds"
          control={
            <FieldText
              aria-label="Foreground timeout seconds"
              disabled={saving}
              inputMode="numeric"
              type="number"
              value={String(draft.execution.foreground_timeout_secs)}
              onChange={(value) =>
                setDraft({
                  ...draft,
                  execution: {
                    ...draft.execution,
                    foreground_timeout_secs: Number(value),
                  },
                })
              }
            />
          }
        />
        <SettingItem
          title="Output line limit"
          control={
            <FieldText
              aria-label="Output line limit"
              disabled={saving}
              inputMode="numeric"
              type="number"
              value={String(draft.execution.output_limit_lines)}
              onChange={(value) =>
                setDraft({
                  ...draft,
                  execution: {
                    ...draft.execution,
                    output_limit_lines: Number(value),
                  },
                })
              }
            />
          }
        />
      </SettingsGroup>

      <SettingsGroup title="Recent decisions">
        <Flex direction="column" gap="2" align="stretch">
          <Flex align="center" gap="2">
            <Button
              loading={auditQuery.isFetching}
              size="sm"
              variant="soft"
              onClick={() => void auditQuery.refetch()}
            >
              Refresh
            </Button>
            {auditQuery.isError ? (
              <Text color="red">Failed to load recent decisions.</Text>
            ) : null}
          </Flex>
          <DataTable<ShellAuditEntry>
            columns={[
              {
                id: "time",
                header: "Time",
                cell: (entry) => formatRelativeTime(entry.ts_ms),
              },
              {
                id: "command",
                header: "Command",
                cell: (entry) => (
                  <code title={entry.command}>
                    {commandPreview(entry.command)}
                  </code>
                ),
              },
              {
                id: "decision",
                header: "Decision",
                cell: (entry) => (
                  <Badge tone={decisionTone(entry.decision)}>
                    {entry.decision}
                  </Badge>
                ),
              },
              {
                id: "rule",
                header: "Layer and rule",
                cell: (entry) => (
                  <span>
                    {entry.layer} · <code>{entry.rule}</code>
                  </span>
                ),
              },
              {
                id: "action",
                header: "Action",
                cell: (entry) => {
                  if (entry.decision !== "pass") return null;
                  const segment = firstCommandSegment(entry.command);
                  const rule = segment ? `argv:${segment}` : undefined;
                  return (
                    <Button
                      disabled={!rule || draft.ask.includes(rule)}
                      size="sm"
                      variant="soft"
                      onClick={() => {
                        if (rule)
                          setDraft({ ...draft, ask: [...draft.ask, rule] });
                      }}
                    >
                      Always ask for this
                    </Button>
                  );
                },
              },
            ]}
            emptyMessage={
              auditQuery.isLoading
                ? "Loading recent decisions…"
                : "No shell commands have been evaluated yet."
            }
            getRowId={(entry, index) =>
              `${String(entry.ts_ms)}-${entry.chat_id}-${String(index)}`
            }
            rows={auditQuery.data?.entries ?? []}
            wide
          />
        </Flex>
      </SettingsGroup>

      <Flex
        align="center"
        className={styles.footer}
        gap="2"
        justify="end"
        wrap="wrap"
      >
        <Button loading={saving} onClick={() => void updatePolicy(draft)}>
          Save
        </Button>
        <SaveStatus state={saveStatus} />
        <Text color="gray">
          Saved to .refact/shell_policy.yaml — commit it to share with your
          team.
        </Text>
      </Flex>
    </SettingsSection>
  );
}
