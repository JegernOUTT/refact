import { useState } from "react";
import { Badge, Button, Icon } from "../../ui";
import { agenticIcon } from "./agenticIcons";
import { ToolCard } from "../../ChatContent/ToolCard";
import { useStoredOpen } from "../../ChatContent/useStoredOpen";
import type { ToolCall } from "../../../services/refact/types";
import { humanizeIdentifier } from "../../../utils/displayNames";
import { CompactToolDetails } from "./CompactToolDetails";
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
  max_steps: number;
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
    natural(row.max_steps) &&
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

export function LaunchPreviewTool({
  toolCall,
  extra,
  hasResult,
  failed,
}: {
  toolCall: ToolCall;
  extra: unknown;
  hasResult: boolean;
  failed: boolean;
}) {
  const [open, toggle] = useStoredOpen(
    toolCall.id ? `tc:${toolCall.id}` : undefined,
    false,
  );
  const [page, setPage] = useState(0);
  const [toolPage, setToolPage] = useState(0);
  const preview = parse(extra);
  const args = readCompactRecord(toolCall.function.arguments);
  const task =
    typeof args?.task === "string" ? compactText(args.task, 160) : null;
  const available = hasResult && !failed && preview !== null;
  const blocked =
    available &&
    (preview.launch_blocked !== undefined || preview.skipped_count > 0);
  const stateLabel = !hasResult
    ? "Preparing preview"
    : !available
      ? "Preview unavailable"
      : blocked
        ? "Launch blocked"
        : "Preview only";
  const facts = available
    ? [
        { label: "Launch", value: "No child started" },
        { label: "Model", value: preview.effective_model },
        {
          label: blocked ? "Blocked" : "Context",
          value:
            preview.launch_blocked ??
            (preview.skipped_count > 0
              ? `${preview.skipped_count} unresolved context entries`
              : `${preview.estimated_tokens} estimated tokens`),
        },
      ]
    : [
        {
          label: "Preview",
          value: !hasResult
            ? "Preparing preview; no acceptance receipt yet"
            : "Details unavailable; this response does not confirm a launch",
        },
      ];
  const toolIndex = Math.min(
    toolPage,
    Math.max(0, (preview?.effective_tools?.length ?? 0) - 1),
  );
  const index = Math.min(
    page,
    Math.max(0, (preview?.context_sources.length ?? 0) - 1),
  );
  const current = preview?.context_sources.at(index);
  const rows: [string, string | number | undefined][] = current
    ? [
        ["Source kind", humanizeIdentifier(current.type)],
        ["Source path", current.path],
        ["Source message", current.message_id],
        ["Source principal", current.principal],
        ["Content digest", current.digest],
        ["Document", current.slug],
        ["Task", current.task_id],
        ["Label", current.label ?? undefined],
        ["First line", current.line1],
        ["Last line", current.line2],
        ["Observed at (Unix ms)", current.observed_at_ms],
      ]
    : [];
  return (
    <ToolCard
      icon={<Icon icon={agenticIcon("spawn")} size="sm" />}
      summary={task ?? "Inspect child launch request"}
      meta={stateLabel}
      density="compact"
      status={!hasResult ? "running" : failed || !preview ? "error" : "success"}
      isOpen={open}
      onToggle={toggle}
      toolCall={toolCall}
    >
      <CompactToolDetails facts={facts}>
        <section aria-label="Child launch preview" className={styles.section}>
          {!hasResult ? (
            <p role="status">
              Preparing launch preview. No acceptance receipt yet.
            </p>
          ) : failed || !preview ? (
            <p role="alert">
              Preview details unavailable. This error or legacy response does
              not confirm a launch.
            </p>
          ) : (
            <>
              <div className={styles.metaRow}>
                <Badge tone="muted">Preview only</Badge>
                <Badge tone={preview.skipped_count ? "warning" : "muted"}>
                  {preview.skipped_count
                    ? "Unresolved context"
                    : "Context resolved"}
                </Badge>
              </div>
              <p className={styles.note}>
                No child spawned. This historical preview is not approval or
                acceptance; run requests revalidate context and authority.
              </p>
              <dl className={styles.detailGrid}>
                {(
                  [
                    ["Effective model", preview.effective_model],
                    ["Estimated context tokens", preview.estimated_tokens],
                    ["Serialized context bytes", preview.serialized_bytes],
                    ["Unresolved entries", preview.skipped_count],
                  ] satisfies [string, string | number][]
                ).map(([label, value]) => (
                  <div key={label} className={styles.detailItem}>
                    <dt className={styles.detailLabel}>{label}</dt>
                    <dd className={styles.detailValue}>{bounded(value)}</dd>
                  </div>
                ))}
              </dl>
              <section
                aria-label="Preview tool authority"
                className={styles.section}
              >
                {preview.effective_tools === undefined ? (
                  <p>
                    Effective tool authority was not recorded in this preview.
                  </p>
                ) : preview.effective_tools.length === 0 ? (
                  <p>No tools permitted by this preview.</p>
                ) : (
                  <>
                    <p className={styles.detailValue}>
                      {bounded(preview.effective_tools[toolIndex])}
                    </p>
                    {preview.effective_tools.length > 1 && (
                      <nav
                        aria-label="Preview tools"
                        className={styles.actions}
                      >
                        <Button
                          size="sm"
                          disabled={toolIndex === 0}
                          onClick={() => setToolPage(toolIndex - 1)}
                        >
                          Previous tool
                        </Button>
                        <span role="status">
                          {toolIndex + 1} of {preview.effective_tools.length}
                        </span>
                        <Button
                          size="sm"
                          disabled={
                            toolIndex + 1 === preview.effective_tools.length
                          }
                          onClick={() => setToolPage(toolIndex + 1)}
                        >
                          Next tool
                        </Button>
                      </nav>
                    )}
                  </>
                )}
                <p className={styles.note}>
                  Concrete tool permissions may narrow before dispatch. They do
                  not grant shell sandbox, file access, or MCP destination
                  permission.
                </p>
              </section>
              <section
                aria-label="Preview deliverable contract"
                className={styles.section}
              >
                {preview.deliverable ? (
                  <dl className={styles.detailGrid}>
                    <div className={styles.detailItem}>
                      <dt className={styles.detailLabel}>Required sections</dt>
                      <dd className={styles.detailValue}>
                        {preview.deliverable.required_sections.length === 0
                          ? "none"
                          : bounded(
                              preview.deliverable.required_sections.join(", "),
                            )}
                      </dd>
                    </div>
                    <div className={styles.detailItem}>
                      <dt className={styles.detailLabel}>Output schema</dt>
                      <dd className={styles.detailValue}>
                        {preview.deliverable.output_schema
                          ? "declared (structural subset enforced)"
                          : "not declared"}
                      </dd>
                    </div>
                    <div className={styles.detailItem}>
                      <dt className={styles.detailLabel}>Evidence</dt>
                      <dd className={styles.detailValue}>
                        {preview.deliverable.evidence_required
                          ? "required section; unbacked claims stay unverified"
                          : "not required"}
                      </dd>
                    </div>
                  </dl>
                ) : (
                  <p>No deliverable contract declared for this preview.</p>
                )}
              </section>
              <section
                aria-label="Effective launch settings"
                className={styles.section}
              >
                {preview.launch_blocked !== undefined ? (
                  <p role="status" data-testid="launch-blocked">
                    Launch blocked: {bounded(preview.launch_blocked)}
                  </p>
                ) : preview.effective_launch ? (
                  <dl className={styles.detailGrid}>
                    {(
                      [
                        [
                          "Revision",
                          `${preview.effective_launch.revision} (schema ${preview.effective_launch.schema_version})`,
                        ],
                        ["Mode", preview.effective_launch.mode],
                        [
                          "Tool authority",
                          toolsLabel(preview.effective_launch.tools),
                        ],
                        ["Step budget", preview.effective_launch.max_steps],
                        [
                          "Nesting depth",
                          preview.effective_launch.subchat_depth,
                        ],
                        ["Checkout", preview.effective_launch.worktree_mode],
                        [
                          "Approval",
                          preview.effective_launch.autonomous_no_confirm
                            ? "autonomous"
                            : [
                                preview.effective_launch
                                  .auto_approve_editing_tools
                                  ? "edits auto-approved"
                                  : "edits confirmed",
                                preview.effective_launch
                                  .auto_approve_dangerous_commands
                                  ? "commands auto-approved"
                                  : "commands confirmed",
                              ].join(", "),
                        ],
                        [
                          "Accepted payload",
                          `${preview.effective_launch.accepted_bytes} of ${preview.effective_launch.accepted_byte_limit} bytes, ${preview.effective_launch.messages} messages`,
                        ],
                      ] as const
                    ).map(([label, value]) => (
                      <div className={styles.detailItem} key={label}>
                        <dt className={styles.detailLabel}>{label}</dt>
                        <dd className={styles.detailValue}>{bounded(value)}</dd>
                      </div>
                    ))}
                  </dl>
                ) : (
                  <p>Effective launch settings were not reported.</p>
                )}
              </section>
              {preview.skipped_count > 0 && (
                <p role="status">
                  Unresolved requested context prevents launch; revise the
                  request before running.
                </p>
              )}
              {current ? (
                <section
                  aria-label="Launch context source"
                  className={styles.section}
                >
                  <dl className={styles.detailGrid}>
                    {rows.flatMap(([label, value]) =>
                      value === undefined
                        ? []
                        : [
                            <div key={label} className={styles.detailItem}>
                              <dt className={styles.detailLabel}>{label}</dt>
                              <dd className={styles.detailValue}>
                                {bounded(value)}
                              </dd>
                            </div>,
                          ],
                    )}
                  </dl>
                  <p className={styles.note}>
                    Source provenance is not permission to read or execute.
                    Historical goals and plans do not activate child control
                    state.
                  </p>
                </section>
              ) : (
                <p>No explicit context sources in this preview.</p>
              )}
              {preview.context_sources.length > 1 && (
                <nav
                  aria-label="Launch context sources"
                  className={styles.actions}
                >
                  <Button
                    size="sm"
                    disabled={index === 0}
                    onClick={() => setPage(index - 1)}
                  >
                    Previous source
                  </Button>
                  <span role="status">
                    {index + 1} of {preview.context_sources.length}
                  </span>
                  <Button
                    size="sm"
                    disabled={index + 1 === preview.context_sources.length}
                    onClick={() => setPage(index + 1)}
                  >
                    Next source
                  </Button>
                </nav>
              )}
            </>
          )}
        </section>
      </CompactToolDetails>
    </ToolCard>
  );
}
