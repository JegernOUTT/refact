import { useState } from "react";
import { Button } from "../../ui";
import { agenticIcon } from "./agenticIcons";
import {
  TranscriptFacts,
  TranscriptItem,
  type TranscriptRawBlock,
  type TranscriptStatus,
} from "../../ChatContent/TranscriptItem";
import { useUnrecordedResultState } from "../../ChatContent/TranscriptItem/useUnrecordedResultState";
import type { ToolCall, ToolResult } from "../../../services/refact/types";
import { humanizeIdentifier } from "../../../utils/displayNames";
import { compactText, readCompactRecord } from "./compactPresentation";
import styles from "./AgenticTool.module.css";

type Source = {
  type: string;
  path?: string;
  message_id?: string;
  principal?: string;
  digest?: string;
  label?: string | null;
  slug?: string;
  task_id?: string;
  observed_at_ms: number;
  line1?: number;
  line2?: number;
};

type Preview = {
  preview: true;
  estimated_tokens: number;
  serialized_bytes: number;
  skipped_count: number;
  effective_model: string;
  effective_tools?: string[];
  deliverable?: {
    required_sections: string[];
    output_schema?: Record<string, unknown> | null;
    evidence_required: boolean;
  };
  context_sources: Source[];
  effective_launch?: EffectiveLaunch;
  launch_blocked?: string;
};

type EffectiveLaunch = {
  schema_version: number;
  revision: number;
  model: string;
  mode: string;
  tools: "All" | "None" | { Only: string[] };
  max_steps: number | null;
  subchat_depth: number;
  stateful: boolean;
  autonomous_no_confirm: boolean;
  auto_approve_editing_tools: boolean;
  auto_approve_dangerous_commands: boolean;
  worktree_mode: string;
  parent_worktree_id?: string | null;
  project_root: string;
  messages: number;
  accepted_bytes: number;
  accepted_byte_limit: number;
};

function effectiveLaunch(value: unknown): value is EffectiveLaunch | undefined {
  if (value === undefined) return true;
  if (typeof value !== "object" || value === null) return false;
  const row = value as Record<string, unknown>;
  const tools = row.tools;
  const toolsValid =
    tools === "All" ||
    tools === "None" ||
    (typeof tools === "object" &&
      tools !== null &&
      Array.isArray((tools as Record<string, unknown>).Only) &&
      ((tools as Record<string, unknown>).Only as unknown[]).every(
        (name) => typeof name === "string" && name.length > 0,
      ));
  return (
    natural(row.schema_version) &&
    natural(row.revision) &&
    typeof row.model === "string" &&
    typeof row.mode === "string" &&
    toolsValid &&
    (row.max_steps === null || natural(row.max_steps)) &&
    natural(row.subchat_depth) &&
    [
      "stateful",
      "autonomous_no_confirm",
      "auto_approve_editing_tools",
      "auto_approve_dangerous_commands",
    ].every((key) => typeof row[key] === "boolean") &&
    typeof row.worktree_mode === "string" &&
    (row.parent_worktree_id === undefined ||
      row.parent_worktree_id === null ||
      typeof row.parent_worktree_id === "string") &&
    typeof row.project_root === "string" &&
    natural(row.messages) &&
    natural(row.accepted_bytes) &&
    natural(row.accepted_byte_limit)
  );
}

function natural(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function source(value: unknown): value is Source {
  if (typeof value !== "object" || value === null) return false;
  const row = value as Record<string, unknown>;
  if (
    typeof row.type !== "string" ||
    ![
      "raw_text",
      "message_range",
      "completed_prefix",
      "file_range",
      "task_doc",
      "memory_ref",
      "parent_instructions",
      "parent_goal",
      "parent_plan",
    ].includes(row.type) ||
    !natural(row.observed_at_ms)
  )
    return false;
  if (
    ["path", "message_id", "principal", "digest", "slug", "task_id"].some(
      (key) => row[key] !== undefined && typeof row[key] !== "string",
    ) ||
    !(
      row.label === undefined ||
      row.label === null ||
      typeof row.label === "string"
    ) ||
    !(row.line1 === undefined || (natural(row.line1) && row.line1 > 0)) ||
    !(row.line2 === undefined || (natural(row.line2) && row.line2 > 0))
  )
    return false;
  return !(natural(row.line1) && natural(row.line2) && row.line2 < row.line1);
}

function deliverable(value: unknown): value is Preview["deliverable"] {
  if (value === undefined) return true;
  if (typeof value !== "object" || value === null) return false;
  const row = value as Record<string, unknown>;
  return (
    Array.isArray(row.required_sections) &&
    row.required_sections.every(
      (name: unknown) => typeof name === "string" && name.length > 0,
    ) &&
    (row.output_schema === undefined ||
      row.output_schema === null ||
      (typeof row.output_schema === "object" &&
        !Array.isArray(row.output_schema))) &&
    (row.evidence_required === undefined ||
      typeof row.evidence_required === "boolean")
  );
}

function parse(value: unknown): Preview | null {
  if (typeof value !== "object" || value === null) return null;
  const row = value as Record<string, unknown>;
  if (
    row.preview !== true ||
    !natural(row.estimated_tokens) ||
    !natural(row.serialized_bytes) ||
    !natural(row.skipped_count) ||
    typeof row.effective_model !== "string" ||
    !row.effective_model ||
    (row.effective_tools !== undefined &&
      (!Array.isArray(row.effective_tools) ||
        !row.effective_tools.every(
          (name: unknown) => typeof name === "string" && name.length > 0,
        ))) ||
    !deliverable(row.deliverable) ||
    !Array.isArray(row.context_sources) ||
    !row.context_sources.every(source) ||
    !effectiveLaunch(row.effective_launch) ||
    !(
      row.launch_blocked === undefined || typeof row.launch_blocked === "string"
    )
  )
    return null;
  return row as Preview;
}

function toolsLabel(tools: EffectiveLaunch["tools"]): string {
  if (tools === "All") return "open (legacy)";
  if (tools === "None") return "none";
  return `${tools.Only.length} concrete`;
}

function bounded(value: string | number): string {
  const text = String(value);
  return text.length > 240 ? `${text.slice(0, 240)}…` : text;
}

type Fact = { label: string; value: string | number | undefined };

function Facts({ label, facts }: { label: string; facts: Fact[] }) {
  return (
    <section aria-label={label} className={styles.section}>
      <TranscriptFacts
        facts={facts.flatMap((fact) =>
          fact.value === undefined
            ? []
            : [{ label: fact.label, value: bounded(fact.value) }],
        )}
      />
    </section>
  );
}

function launchFacts(launch: EffectiveLaunch): Fact[] {
  return [
    {
      label: "Revision",
      value: `${launch.revision} (schema ${launch.schema_version})`,
    },
    { label: "Mode", value: launch.mode },
    { label: "Tool authority", value: toolsLabel(launch.tools) },
    { label: "Step budget", value: launch.max_steps ?? "Unlimited" },
    { label: "Nesting depth", value: launch.subchat_depth },
    { label: "Checkout", value: launch.worktree_mode },
    {
      label: "Approval",
      value: launch.autonomous_no_confirm
        ? "autonomous"
        : [
            launch.auto_approve_editing_tools
              ? "edits auto-approved"
              : "edits confirmed",
            launch.auto_approve_dangerous_commands
              ? "commands auto-approved"
              : "commands confirmed",
          ].join(", "),
    },
    {
      label: "Accepted payload",
      value: `${launch.accepted_bytes} of ${launch.accepted_byte_limit} bytes, ${launch.messages} messages`,
    },
  ];
}

function sourceFacts(current: Source): Fact[] {
  return [
    { label: "Source kind", value: humanizeIdentifier(current.type) },
    { label: "Source path", value: current.path },
    { label: "Source message", value: current.message_id },
    { label: "Source principal", value: current.principal },
    { label: "Content digest", value: current.digest },
    { label: "Document", value: current.slug },
    { label: "Task", value: current.task_id },
    { label: "Label", value: current.label ?? undefined },
    { label: "First line", value: current.line1 },
    { label: "Last line", value: current.line2 },
    { label: "Observed at (Unix ms)", value: current.observed_at_ms },
  ];
}

function Pager({
  label,
  index,
  total,
  noun,
  onChange,
}: {
  label: string;
  index: number;
  total: number;
  noun: string;
  onChange: (index: number) => void;
}) {
  if (total < 2) return null;
  return (
    <nav aria-label={label} className={styles.actions}>
      <Button
        size="sm"
        disabled={index === 0}
        onClick={() => onChange(index - 1)}
      >
        Previous {noun}
      </Button>
      <span role="status">
        {index + 1} of {total}
      </span>
      <Button
        size="sm"
        disabled={index + 1 === total}
        onClick={() => onChange(index + 1)}
      >
        Next {noun}
      </Button>
    </nav>
  );
}

function PreviewInput({ preview }: { preview: Preview }) {
  const [page, setPage] = useState(0);
  const [toolPage, setToolPage] = useState(0);
  const tools = preview.effective_tools;
  const toolIndex = Math.min(toolPage, Math.max(0, (tools?.length ?? 0) - 1));
  const index = Math.min(page, Math.max(0, preview.context_sources.length - 1));
  const current = preview.context_sources.at(index);
  const contract = preview.deliverable;
  return (
    <section aria-label="Child launch preview" className={styles.section}>
      <section aria-label="Preview tool authority" className={styles.section}>
        {tools === undefined ? (
          <p>Tool authority not recorded.</p>
        ) : tools.length === 0 ? (
          <p>No tools permitted.</p>
        ) : (
          <>
            <p>{bounded(tools[toolIndex])}</p>
            <Pager
              label="Preview tools"
              index={toolIndex}
              total={tools.length}
              noun="tool"
              onChange={setToolPage}
            />
          </>
        )}
      </section>
      {contract ? (
        <Facts
          label="Preview deliverable contract"
          facts={[
            {
              label: "Required sections",
              value:
                contract.required_sections.length === 0
                  ? "none"
                  : contract.required_sections.join(", "),
            },
            {
              label: "Output schema",
              value: contract.output_schema ? "declared" : "not declared",
            },
            {
              label: "Evidence",
              value: contract.evidence_required ? "required" : "not required",
            },
          ]}
        />
      ) : null}
      {preview.launch_blocked === undefined && preview.effective_launch && (
        <Facts
          label="Effective launch settings"
          facts={launchFacts(preview.effective_launch)}
        />
      )}
      {current && (
        <Facts label="Launch context source" facts={sourceFacts(current)} />
      )}
      <Pager
        label="Launch context sources"
        index={index}
        total={preview.context_sources.length}
        noun="source"
        onChange={setPage}
      />
    </section>
  );
}

export function LaunchPreviewTool({
  toolCall,
  rawContent,
  extra,
  hasResult,
  failed,
}: {
  toolCall: ToolCall;
  rawContent?: ToolResult["content"];
  extra: unknown;
  hasResult: boolean;
  failed: boolean;
}) {
  const unrecorded = useUnrecordedResultState(toolCall.id);
  const preview = parse(extra);
  const args = readCompactRecord(toolCall.function.arguments);
  const task =
    typeof args?.task === "string" ? compactText(args.task, 160) : null;
  const available = hasResult && !failed && preview !== null;
  const blockedReason = !available
    ? null
    : preview.launch_blocked ??
      (preview.skipped_count > 0
        ? `${preview.skipped_count} unresolved context ${
            preview.skipped_count === 1 ? "entry" : "entries"
          }`
        : null);
  const status: TranscriptStatus = !hasResult
    ? unrecorded.status
    : !available
      ? "failed"
      : blockedReason !== null
        ? "needs_you"
        : "done";
  const raw: TranscriptRawBlock[] = [
    { label: "Arguments", value: toolCall.function.arguments },
  ];
  if (rawContent !== undefined)
    raw.push({ label: "Response", value: rawContent });
  if (extra !== undefined) raw.push({ label: "Metadata", value: extra });
  const Icon = agenticIcon("spawn");
  return (
    <TranscriptItem
      data-testid="launch-preview"
      icon={<Icon aria-hidden />}
      verb="Launch"
      target={task ?? undefined}
      meta="preview"
      status={status}
      statusLabel={
        !hasResult
          ? unrecorded.statusLabel
          : blockedReason !== null
            ? "Blocked"
            : undefined
      }
      summary={
        available
          ? [
              { label: "Model", value: bounded(preview.effective_model) },
              {
                label: "Context",
                value: `${preview.estimated_tokens} tokens · ${preview.serialized_bytes} bytes`,
              },
              ...(preview.skipped_count > 0
                ? [
                    {
                      label: "Unresolved",
                      value: String(preview.skipped_count),
                    },
                  ]
                : []),
            ]
          : []
      }
      input={available ? <PreviewInput preview={preview} /> : undefined}
      error={
        blockedReason !== null ? (
          <span data-testid="launch-blocked">
            Launch blocked: {bounded(blockedReason)}
          </span>
        ) : hasResult && !available ? (
          "Preview unavailable; this response does not confirm a launch."
        ) : undefined
      }
      raw={raw}
      openKey={toolCall.id ? `launch_preview:${toolCall.id}` : undefined}
      defaultOpen={status === "needs_you"}
    />
  );
}
