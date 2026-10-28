import {
  isWorkRow,
  parseWorkPage,
  type WorkRow,
} from "../../../services/refact/workTypes";
import {
  humanizeIdentifier,
  stripAgentKindPrefix,
} from "../../../utils/displayNames";
import { normalizeToolName } from "../../../utils/toolNameAliases";
import { parseAgentStatusOutput } from "../../ChatContent/AgentStatusModel";
import { buildAgenticToolViewModel } from "./agenticToolModel";
import { parseApprovalReceipt } from "./approvalReceipt";
import {
  COMPACT_HEADLINE_CHARS,
  compactFacts,
  compactText,
  readCompactRecord,
  readCompactText,
  type CompactAgenticInput,
  type CompactAgenticPresentation,
} from "./compactPresentation";
import { parseFamilyReport } from "./familyReport";
import { parseResumeReceipt } from "./resumeReceipt";

type State = Pick<CompactAgenticPresentation, "stateLabel" | "tone">;
type Fact = { label: string; value: unknown };

const OPERATIONS: Partial<Record<string, string>> = {
  agents_overview: "Check the agent tree",
  agent_list: "List background work",
  work_overview: "Check project work",
  agent_status: "Check child status",
  agent_wait: "Wait for child work",
  agent_result: "Read the child result",
  work_inspect: "Inspect work",
  wait_agents: "Wait for card agents",
  agent_chat_summary: "Read the agent conversation summary",
  task_overview: "Check task activity",
  task_questions_list: "Check planner questions",
  agent_cancel: "Request child cancellation",
  agent_resume: "Request child continuation",
  agent_lifecycle: "Manage child lifecycle",
  wait: "Wait for execution",
  schedule: "Manage scheduled work",
  pause: "Request an execution hold",
  resume: "Release an execution hold",
  agent_approval: "Review child command permission",
  pause_agent: "Request a pause",
  resume_agent: "Request continuation",
  restart_agent: "Request a restart",
  cancel_agent: "Request cancellation",
  assign_agent: "Assign card work",
  merge_agent: "Request a local merge",
  merge_ready_in_order: "Merge ready cards locally",
  pick_ab_winner: "Choose an A/B variant",
};

const STATES: Partial<Record<string, State>> = {
  queued: { stateLabel: "Queued", tone: "neutral" },
  running: { stateLabel: "Running snapshot", tone: "active" },
  waiting_for_approval: { stateLabel: "Approval needed", tone: "warning" },
  waiting_user: { stateLabel: "Needs user input", tone: "warning" },
  paused: { stateLabel: "Paused", tone: "warning" },
  parked: { stateLabel: "Parked", tone: "warning" },
  idle: { stateLabel: "Idle snapshot", tone: "neutral" },
  completed: { stateLabel: "Completed · not verified", tone: "neutral" },
  failed: { stateLabel: "Failed snapshot", tone: "danger" },
  cancelled: { stateLabel: "Cancelled", tone: "neutral" },
  interrupted: { stateLabel: "Interrupted", tone: "warning" },
  closed: { stateLabel: "Closed · resumable", tone: "neutral" },
  unknown: { stateLabel: "State unknown", tone: "neutral" },
};

const STATUS_CHIPS: Partial<Record<string, string>> = {
  "⏳ queued": "queued",
  "🟢 running": "running",
  "✋ waiting_for_approval": "waiting_for_approval",
  "✅ completed": "completed",
  "❌ failed": "failed",
  "⏹ cancelled": "cancelled",
  "⚠ interrupted": "interrupted",
  "🔒 closed": "closed",
};

function targetState(value: unknown): State | null {
  if (typeof value !== "string") return null;
  const key = Object.prototype.hasOwnProperty.call(STATUS_CHIPS, value)
    ? STATUS_CHIPS[value] ?? value
    : value;
  if (value.startsWith("⏸ paused (by ") && value.endsWith(")"))
    return STATES.paused ?? null;
  return Object.prototype.hasOwnProperty.call(STATES, key)
    ? STATES[key] ?? null
    : null;
}

function snapshotState(values: readonly unknown[]): State {
  const states = values.map(targetState);
  const labels = new Set(
    states.map((state) => state?.stateLabel ?? "State unknown"),
  );
  if (values.length === 1 && states[0] !== null) return states[0];
  const attention = states.filter(
    (state): state is State =>
      state !== null && (state.tone === "danger" || state.tone === "warning"),
  );
  if (attention.length > 0) {
    const failed = attention.some((state) => state.tone === "danger");
    const approval = attention.some(
      (state) => state.stateLabel === "Approval needed",
    );
    const held = attention.some(
      (state) =>
        state.tone === "warning" && state.stateLabel !== "Approval needed",
    );
    return {
      stateLabel: [
        failed && "Failures",
        approval && "Approval needed",
        held && "Held work",
      ]
        .filter(Boolean)
        .join(" · "),
      tone: failed ? "danger" : "warning",
    };
  }
  if (values.length === 0)
    return { stateLabel: "Snapshot received", tone: "neutral" };
  if (labels.size === 1)
    return states[0] ?? { stateLabel: "State unknown", tone: "neutral" };
  return { stateLabel: "Mixed / unknown states", tone: "neutral" };
}

function contentText(value: unknown): string | null {
  if (typeof value === "string") return value;
  if (value === undefined || value === null) return null;
  try {
    return JSON.stringify(value);
  } catch {
    return null;
  }
}

function requestState(input: CompactAgenticInput): State {
  switch (input.transportState) {
    case "pending":
      return { stateLabel: "Awaiting result", tone: "neutral" };
    case "running":
      return { stateLabel: "Request in progress", tone: "active" };
    case "failed":
      return { stateLabel: "Request failed · target unknown", tone: "danger" };
    case "succeeded":
      return { stateLabel: "Response received · unconfirmed", tone: "neutral" };
    default:
      return { stateLabel: "Outcome unknown", tone: "neutral" };
  }
}

function workTitle(row: WorkRow): string | null {
  return row.disclosure === "withheld"
    ? "Work withheld by privacy policy"
    : compactText(row.title) ?? compactText(row.card_id);
}

function approvalState(
  content: string | null,
  list: boolean,
): {
  state: State;
  facts: Fact[];
} | null {
  const receipts = parseApprovalReceipt(content);
  if (receipts === null) return null;
  const decisions = receipts.map(
    (row) => row.decision ?? (list ? "pending" : "unknown"),
  );
  const rejected = decisions.some(
    (value) => value === "reject" || value === "rejected",
  );
  const pending = decisions.includes("pending");
  let state: State;
  if (rejected || pending) {
    state = {
      stateLabel:
        rejected && pending
          ? "Denied · approval needed"
          : rejected
            ? "Permission denied"
            : "Approval needed",
      tone: "warning",
    };
  } else if (
    decisions.some((value) => value === "expired" || value === "cancelled")
  ) {
    state = { stateLabel: "Permission expired / cancelled", tone: "warning" };
  } else if (
    decisions.length > 0 &&
    decisions.every((value) => value === "approve" || value === "approved")
  ) {
    state = { stateLabel: "Approved once · outcome unknown", tone: "neutral" };
  } else {
    state = {
      stateLabel:
        receipts.length === 0
          ? "No requests in snapshot"
          : "Permission unconfirmed",
      tone: "neutral",
    };
  }
  return {
    state,
    facts: [
      { label: "Snapshot", value: `${receipts.length} command requests shown` },
      {
        label: "Permission",
        value: "A decision is not a command execution result",
      },
      { label: "Scope", value: "Historical receipt · single execution only" },
    ],
  };
}

function agentTitle(title: string | null | undefined): string | null {
  if (!title) return null;
  return stripAgentKindPrefix(title) || null;
}

export function buildCompactWorkControlPresentation(
  input: CompactAgenticInput,
): CompactAgenticPresentation | null {
  const name = normalizeToolName(input.toolName) ?? input.toolName;
  const operation = Object.prototype.hasOwnProperty.call(OPERATIONS, name)
    ? OPERATIONS[name]
    : undefined;
  if (operation === undefined) return null;
  const args = readCompactRecord(input.arguments);
  const result = readCompactRecord(input.result);
  const content = contentText(input.result);
  let verb = operation;
  if (name === "agent_resume" || name === "agent_lifecycle") {
    const actions: Partial<Record<string, string>> = {
      inspect: "Check child continuation options",
      pause: "Request a child pause",
      release: "Release the parent pause",
      close: "Close child execution",
      recover: "Recover child execution",
      continue: "Continue completed child work",
      raise_and_recover: "Raise the child limit and request recovery",
      raise_and_resume: "Raise the child limit and request continuation",
    };
    const action = readCompactText(args, "action");
    if (
      action !== null &&
      Object.prototype.hasOwnProperty.call(actions, action)
    )
      verb = actions[action] ?? verb;
  }
  if (name === "agent_approval") {
    if (args?.action === "approve") verb = "Request one child command approval";
    if (args?.action === "reject") verb = "Reject a child command request";
  }
  let title = readCompactText(args, "title", "card_title", "card_id");
  let state = requestState(input);
  let facts: Fact[] = [];
  if (input.transportState === "failed") {
    facts.push({
      label: "Target",
      value: "May still be running unchanged; no state change is confirmed",
    });
  }
  if (input.transportState === "succeeded") {
    const waiting = name === "agent_wait";
    if (waiting) {
      state = { stateLabel: "Wait ended · state unconfirmed", tone: "neutral" };
      if (result?.timed_out !== true)
        facts.push({
          label: "Wait",
          value:
            "The wait ended; this does not mean the child failed or stopped",
        });
    }
    const page =
      name === "work_overview" || name === "work_inspect"
        ? parseWorkPage(result)
        : null;
    if (page !== null) {
      const detail =
        name === "work_inspect" && isWorkRow(result?.row) ? result.row : null;
      if (detail !== null) title = workTitle(detail);
      state =
        detail !== null
          ? targetState(detail.state) ?? state
          : snapshotState(page.rows.map((row) => row.state));
      if (
        (detail?.attention === true ||
          page.rows.some((row) => row.attention)) &&
        state.tone !== "danger" &&
        state.tone !== "warning"
      ) {
        state = { stateLabel: "Needs attention", tone: "warning" };
      }
      const unit = page.page_unit === "work" ? "work items" : "worktrees";
      facts = [
        {
          label: "Snapshot",
          value: `${page.returned} of ${page.total} reported ${unit}${
            page.truncated ? " · more pages" : ""
          }`,
        },
        {
          label: "Coverage",
          value: `${
            compactText(page.coverage) ?? "Unknown"
          } · historical snapshot`,
        },
        {
          label: "Provenance",
          value:
            detail !== null
              ? `${humanizeIdentifier(detail.freshness)}${
                  detail.disclosure === "withheld" ? " · privacy-limited" : ""
                }`
              : "Counts describe this permitted page, not all work",
        },
      ];
    } else if (name === "wait_agents") {
      const report = content === null ? null : parseAgentStatusOutput(content);
      if (report !== null) {
        const counts = readCompactRecord(result?.alerts);
        const header = content?.match(
          /^⚠️ {2}Alerts: (\d+) stuck, (\d+) failed, (\d+) needing approval, (\d+) needing attention(?:\r?\n|$)/u,
        );
        const reportedHeader =
          header !== null &&
          header !== undefined &&
          header
            .slice(1)
            .every((value) => Number.isSafeInteger(Number(value))) &&
          Number(header[1]) === report.alerts.stuck &&
          Number(header[2]) === report.alerts.failed;
        const explicitApproval =
          (counts !== null &&
            counts.failed === report.alerts.failed &&
            counts.stuck === report.alerts.stuck &&
            counts.needing_approval === report.alerts.approval) ||
          report.rows.some((row) => row.approvalRequired);
        const reported =
          Object.values(report.alerts).every(
            (count) => Number.isSafeInteger(count) && count >= 0,
          ) &&
          (reportedHeader || explicitApproval);
        const failures = reported && report.alerts.failed > 0;
        const blocked = reported && report.alerts.stuck > 0;
        const held = reported && explicitApproval && report.alerts.approval > 0;
        const alerts = [
          failures && "Failures",
          blocked && "Blocked",
          held && "Approval needed",
        ]
          .filter(Boolean)
          .join(" · ");
        state = {
          stateLabel:
            alerts === "" ? "Wait snapshot · state unconfirmed" : alerts,
          tone: failures ? "danger" : blocked || held ? "warning" : "neutral",
        };
        if (report.rows.length === 1) title ??= report.rows[0].title;
        facts = [
          {
            label: "Snapshot",
            value: `${report.rows.length} card rows shown · coverage unknown`,
          },
          {
            label: "Reported alerts",
            value: reported
              ? `${report.alerts.failed} failed · ${
                  report.alerts.stuck
                } stuck · ${
                  explicitApproval
                    ? `${report.alerts.approval} awaiting approval`
                    : "Approval count unconfirmed"
                }`
              : "No recognized alert counts; target state unknown",
          },
          {
            label: "Wait",
            value:
              "Historical wait request snapshot; not proof of child completion or planner wake-up",
          },
        ];
      }
    } else if (name === "agent_approval") {
      const approval = approvalState(content, args?.action === "list");
      if (approval !== null) ({ state, facts } = approval);
    } else if (name === "agent_resume" || name === "agent_lifecycle") {
      const receipt = parseResumeReceipt(content, name === "agent_lifecycle");
      if (receipt !== null) {
        state = targetState(receipt.status) ?? state;
        const approval = receipt.status === "waiting_for_approval";
        if (receipt.cap_stop !== null) {
          state = {
            stateLabel: approval
              ? "Approval needed · cap reached"
              : receipt.status === "failed"
                ? "Failed · execution cap reached"
                : "Execution cap reached",
            tone: "warning",
          };
        } else if (
          !receipt.resume_configuration_available ||
          !receipt.original_scope_available
        ) {
          state = {
            stateLabel: approval
              ? "Approval needed · resume blocked"
              : receipt.status === "failed"
                ? "Failed · resume unavailable"
                : "Resume unavailable",
            tone: "warning",
          };
        }
        facts = [
          {
            label: "Scope",
            value: "Historical child receipt · not proof of command success",
          },
          {
            label: "Continuation",
            value:
              receipt.action === "inspect"
                ? "Inspection only; execution unchanged"
                : receipt.accepted === false
                  ? receipt.summary ?? "No lifecycle change accepted"
                  : receipt.action === "close"
                    ? "Child closed; not a pause hold"
                    : "Request accepted; queued does not mean running",
          },
          {
            label: "Limits",
            value:
              receipt.cap_stop !== null
                ? `${humanizeIdentifier(receipt.cap_stop.key)}: ${
                    receipt.cap_stop.used
                  } / ${receipt.cap_stop.limit}`
                : !receipt.resume_configuration_available
                  ? "Saved runtime configuration unavailable"
                  : !receipt.original_scope_available
                    ? "Original execution scope unavailable"
                    : "Existing permission and goal ceilings unchanged",
          },
        ];
      }
    } else if (name === "wait") {
      const outcome = readCompactText(result, "outcome");
      const labels: Partial<Record<string, State>> = {
        completed: {
          stateLabel: "Wait completed · target success unconfirmed",
          tone: "neutral",
        },
        timed_out: {
          stateLabel: "Wait timed out · target unchanged",
          tone: "warning",
        },
        interrupted: {
          stateLabel: "Wait interrupted · target unchanged",
          tone: "warning",
        },
        parked: {
          stateLabel: "Planner parked · agents nonterminal",
          tone: "warning",
        },
        no_matching_agents: {
          stateLabel: "No matching agents",
          tone: "neutral",
        },
      };
      if (
        outcome !== null &&
        Object.prototype.hasOwnProperty.call(labels, outcome)
      ) {
        state = labels[outcome] ?? state;
        facts = [
          {
            label: "Wait",
            value:
              "Historical wait receipt; returning does not verify worker success or cancel the target",
          },
        ];
      }
    } else if (name === "schedule") {
      const action = readCompactText(args, "action");
      if (action === "delete" && typeof result?.removed === "boolean") {
        state = {
          stateLabel: result.removed
            ? "Schedule removed"
            : "Schedule not found",
          tone: "neutral",
        };
      } else if (
        action === "update" &&
        result?.updated === true &&
        readCompactText(result, "id") !== null
      ) {
        state = {
          stateLabel: "Schedule updated · execution unconfirmed",
          tone: "neutral",
        };
      } else if (
        action === "create" &&
        readCompactText(result, "id") !== null &&
        readCompactText(result, "human_schedule") !== null
      ) {
        state = {
          stateLabel: "Schedule created · execution unconfirmed",
          tone: "neutral",
        };
      } else if (action === "list" && content !== null) {
        try {
          const rows: unknown = JSON.parse(content);
          if (
            Array.isArray(rows) &&
            rows.every(
              (row) => readCompactText(readCompactRecord(row), "id") !== null,
            )
          ) {
            state = {
              stateLabel: "Schedule snapshot received",
              tone: "neutral",
            };
            facts.push({
              label: "Snapshot",
              value: `${rows.length} schedules shown`,
            });
          }
        } catch {
          state = requestState(input);
        }
      }
      facts.push({
        label: "Schedule",
        value:
          "A schedule receipt does not prove scheduled work ran or succeeded",
      });
    } else if (name === "pause" || name === "resume") {
      const target = readCompactRecord(args?.target);
      if (
        target?.kind === "child" &&
        result?.accepted === true &&
        readCompactText(result, "agent_id") !== null
      ) {
        state = {
          stateLabel:
            name === "pause"
              ? "Child pause requested · parks at next boundary"
              : result.status === "paused"
                ? "Parent hold released · other holds remain"
                : "Parent hold released · execution unconfirmed",
          tone: name === "pause" ? "warning" : "neutral",
        };
      } else if (
        target?.kind === "goal" &&
        result?.target === "goal" &&
        ["paused", "snoozed", "resumed"].includes(String(result.status))
      ) {
        state = {
          stateLabel:
            result.status === "resumed"
              ? "Timed goal hold released"
              : result.status === "snoozed"
                ? "Goal snoozed"
                : "Goal paused",
          tone: result.status === "resumed" ? "neutral" : "warning",
        };
      } else if (
        target?.kind === "schedule" &&
        result?.updated === true &&
        typeof result.enabled === "boolean" &&
        readCompactText(result, "id") !== null
      ) {
        state = {
          stateLabel: result.enabled
            ? "Schedule enabled · execution unconfirmed"
            : "Schedule paused",
          tone: result.enabled ? "neutral" : "warning",
        };
      } else if (
        target?.kind === "card" &&
        typeof target.card_id === "string" &&
        content !== null
      ) {
        if (
          name === "pause" &&
          (content.startsWith(`⏸️ Paused ${target.card_id};`) ||
            content.startsWith(
              `⏸️ Recorded the pause on card ${target.card_id};`,
            ))
        )
          state = { stateLabel: "Card pause recorded", tone: "warning" };
        if (
          name === "resume" &&
          (content.startsWith(`▶️ Resumed ${target.card_id}.`) ||
            content.startsWith(
              `⚠️ Recorded the resume on card ${target.card_id},`,
            ))
        )
          state = {
            stateLabel: "Card hold released · execution unconfirmed",
            tone: "neutral",
          };
      }
      facts.push({
        label: "Hold",
        value:
          "Historical pause-hold receipt; release is not lifecycle recovery or proof of execution",
      });
    } else {
      const model = buildAgenticToolViewModel({
        toolName: name,
        argumentsRaw: undefined,
        content,
        hasResult: true,
      });
      const report = parseFamilyReport(content);
      const roster = name === "agents_overview" || name === "agent_list";
      if (roster && model !== null && model.agents.length > 0) {
        state = snapshotState(model.agents.map((row) => row.status));
        if (model.agents.length === 1)
          title ??= agentTitle(model.agents[0].title);
        facts = [
          {
            label: "Snapshot",
            value: `${model.agents.length} rows shown · historical excerpt`,
          },
          { label: "Coverage", value: "Unknown beyond the returned rows" },
        ];
      } else if (
        name === "agent_status" ||
        name === "agent_result" ||
        name === "agent_wait"
      ) {
        const observed =
          result !== null &&
          readCompactText(result, "agent_id", "card_id") !== null
            ? targetState(result.status)
            : targetState(
                model?.details.find((row) => row.label === "Status")?.value,
              );
        if (observed !== null) {
          state = waiting
            ? {
                ...observed,
                stateLabel: `Wait ended · ${observed.stateLabel.toLowerCase()}`,
              }
            : observed;
          title ??= agentTitle(
            readCompactText(result, "title") ?? report.title,
          );
          facts.push({
            label: "Evidence",
            value: "Historical state snapshot · completion is not verification",
          });
        }
        if (name === "agent_wait" && result?.timed_out === true) {
          state = {
            stateLabel:
              observed?.tone === "danger"
                ? "Wait ended · failure reported"
                : observed?.tone === "warning"
                  ? `Wait ended · ${observed.stateLabel.toLowerCase()}`
                  : "Wait ended · time limit",
            tone: observed?.tone === "danger" ? "danger" : "warning",
          };
          facts.unshift({
            label: "Wait",
            value:
              "The wait ended; this does not mean the child failed or stopped",
          });
        }
      } else if (
        name === "task_overview" ||
        name === "agent_chat_summary" ||
        name === "task_questions_list"
      ) {
        if (report.kind === "markdown") {
          if (name === "task_overview") title ??= report.title;
          if (report.rowTotal > 0)
            facts.push({
              label: "Report",
              value: `${report.rowTotal} rows in the returned excerpt · coverage unknown`,
            });
        }
      } else if (result?.accepted === true) {
        state = {
          stateLabel: "Request accepted · unconfirmed",
          tone: "neutral",
        };
        facts.push({
          label: "Target",
          value: "Acceptance does not prove the requested state change",
        });
      }
      if (name === "merge_agent" || name === "merge_ready_in_order") {
        let rows: unknown = result;
        if (name === "merge_ready_in_order" && content !== null) {
          try {
            rows = JSON.parse(content) as unknown;
          } catch {
            rows = null;
          }
        }
        const entries = Array.isArray(rows) ? rows : [rows];
        const reports = entries.map(readCompactRecord);
        if (
          reports.length > 0 &&
          reports.every(
            (row) =>
              row !== null &&
              typeof row.card_id === "string" &&
              typeof row.merged === "boolean" &&
              typeof row.conflict === "boolean" &&
              (row.error === undefined || typeof row.error === "string"),
          )
        ) {
          if (reports.some((row) => row?.conflict === true))
            state = { stateLabel: "Merge conflict reported", tone: "warning" };
          else if (
            reports.some(
              (row) =>
                typeof row?.error === "string" &&
                compactText(row.error) !== null,
            )
          )
            state = { stateLabel: "Merge error reported", tone: "danger" };
          else if (reports.every((row) => row?.merged === true))
            state = {
              stateLabel: "Merged locally",
              tone: "neutral",
            };
          else state = { stateLabel: "Merge not confirmed", tone: "neutral" };
          facts.unshift({
            label: "Merge",
            value: `${
              reports.filter(
                (row) => row?.merged === true && row.conflict === false,
              ).length
            } local merges reported in ${
              reports.length
            } returned card outcomes · push and verification unconfirmed`,
          });
        }
      }
      if (
        name === "task_questions_list" &&
        report.headers.join("|") ===
          "Card|Title|Question ID|Urgency|Question" &&
        report.rowTotal > 0
      ) {
        state = { stateLabel: "Planner questions pending", tone: "warning" };
        facts.unshift({
          label: "Questions",
          value: `${report.rowTotal} unanswered questions in returned excerpt · coverage unknown`,
        });
      }
    }
    if (
      result !== null &&
      typeof result.error === "string" &&
      compactText(result.error) !== null
    ) {
      state = { stateLabel: "Request error · target unknown", tone: "danger" };
      facts.unshift({ label: "Request error", value: result.error });
    }
  }
  return {
    headline:
      compactText(
        title === null ? verb : `${verb}: ${title}`,
        COMPACT_HEADLINE_CHARS,
      ) ?? verb,
    ...state,
    facts: compactFacts(facts),
  };
}
