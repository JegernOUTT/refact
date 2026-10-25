import {
  goalEvidenceVerdict,
  parseEvidenceVerdict,
  type EvidenceVerdict,
} from "../../../services/refact/evidenceTypes";
import { humanizeIdentifier } from "../../../utils/displayNames";
import { normalizeToolName } from "../../../utils/toolNameAliases";
import {
  COMPACT_HEADLINE_CHARS,
  compactFacts,
  compactText,
  compactTransportPresentation,
  readCompactRecord,
  type CompactAgenticInput,
  type CompactAgenticPresentation,
} from "./compactPresentation";

type CompactState = Pick<CompactAgenticPresentation, "stateLabel" | "tone">;
type RecordValue = Readonly<Record<string, unknown>>;

const VERDICTS: Record<EvidenceVerdict, CompactState> = {
  passed: { stateLabel: "Criteria met", tone: "positive" },
  failed: { stateLabel: "Criteria not met", tone: "danger" },
  blocked: { stateLabel: "Validation blocked", tone: "warning" },
  unavailable: { stateLabel: "Validation unavailable", tone: "warning" },
  not_run: { stateLabel: "Validation not run", tone: "neutral" },
  superseded: { stateLabel: "Verdict superseded", tone: "warning" },
};

const GOAL_STATES = {
  active: { stateLabel: "Active (snapshot)", tone: "active" },
  verifying: { stateLabel: "Verifying (snapshot)", tone: "active" },
  paused: { stateLabel: "Paused (snapshot)", tone: "warning" },
  completed: { stateLabel: "Completed (snapshot)", tone: "neutral" },
  stopped: { stateLabel: "Stopped (snapshot)", tone: "neutral" },
  budget_exhausted: {
    stateLabel: "Budget exhausted (snapshot)",
    tone: "warning",
  },
  no_progress: { stateLabel: "No progress (snapshot)", tone: "warning" },
  transferred: { stateLabel: "Transferred (snapshot)", tone: "neutral" },
} satisfies Record<string, CompactState>;

type GoalState = keyof typeof GOAL_STATES;

function record(value: unknown): RecordValue | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as RecordValue)
    : null;
}

function natural(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function positive(value: unknown): value is number {
  return natural(value) && value > 0;
}

function goalState(value: unknown): value is GoalState {
  return (
    typeof value === "string" &&
    Object.prototype.hasOwnProperty.call(GOAL_STATES, value)
  );
}

function headline(value: unknown): string | null {
  if (typeof value !== "string") return null;
  for (const line of value.split(/\r?\n/u)) {
    const text = line.replace(/^\s*(?:#{1,6}\s+|[-*+]\s+|\d+[.)]\s+)/u, "");
    if (/^\s*(?:(?:goal|plan)(?: updates)?|[-_*]{3,})\s*$/iu.test(text))
      continue;
    const bounded = compactText(text, COMPACT_HEADLINE_CHARS);
    if (bounded !== null) return bounded;
  }
  return null;
}

function intent(tool: string, args: RecordValue | null): string {
  switch (tool) {
    case "set_plan":
      return (
        headline(args?.summary) ??
        headline(args?.content) ??
        headline(args?.path) ??
        "A plan for the work ahead"
      );
    case "update_plan":
      return headline(args?.note) ?? "Changes to the current plan";
    case "get_plan":
      return "The current plan";
    case "set_goal":
      return headline(args?.content) ?? "A goal for this chat";
    case "update_goal":
      return headline(args?.note) ?? "Changes to the current goal";
    case "get_goal":
      return "The current goal";
    case "validate_goal":
      return "Check the goal’s success criteria";
    case "pause_goal":
      return headline(args?.reason) ?? "Hold goal pursuit until resumed";
    default:
      return headline(args?.reason) ?? "Hold goal pursuit briefly";
  }
}

function waitSubject(value: unknown): string | null {
  const wait = record(value);
  const subject = record(wait?.subject);
  if (
    wait === null ||
    subject === null ||
    (wait.since_ms !== undefined && !natural(wait.since_ms)) ||
    (wait.deadline_ms !== undefined &&
      wait.deadline_ms !== null &&
      !natural(wait.deadline_ms))
  )
    return null;
  switch (subject.kind) {
    case "process":
      return typeof subject.process_id === "string" && subject.process_id !== ""
        ? "Waiting on a process"
        : null;
    case "child":
      return typeof subject.agent_id === "string" && subject.agent_id !== ""
        ? "Waiting on a child"
        : null;
    case "event":
      return typeof subject.name === "string" && subject.name !== ""
        ? "Waiting on an event"
        : null;
    case "deadline":
      return natural(subject.until_ms) ? "Waiting on a deadline" : null;
    default:
      return null;
  }
}

function budgetText(value: unknown): string | null {
  const budget = record(value);
  if (budget === null) return null;
  const limits = [
    ["max_turns", "turns"],
    ["max_tokens", "tokens"],
    ["max_minutes", "minutes"],
    ["max_cost_cents", "cents"],
    ["no_progress_turns", "no-progress turns"],
  ] as const;
  const finite: string[] = [];
  for (const [key, unit] of limits) {
    const limit = budget[key];
    if (limit === null || limit === undefined || limit === 0) continue;
    if (!positive(limit)) return null;
    finite.push(`${limit} ${unit}`);
  }
  return finite.length === 0 ? "No budget limits" : finite.join(" · ");
}

function latestValidation(value: unknown): string {
  if (value === null) return "No verdict";
  if (value === "met") return "Criteria met (reported)";
  if (value === "unmet") return "Criteria not met";
  if (value === "wait") return "Waiting; no pass";
  const verdict = parseEvidenceVerdict(value);
  return verdict === null ? "Verdict unknown" : VERDICTS[verdict].stateLabel;
}

function goalPresentation(
  goal: RecordValue,
): CompactAgenticPresentation | null {
  if (
    typeof goal.content !== "string" ||
    !goalState(goal.status) ||
    !positive(goal.version) ||
    !natural(goal.delta_count) ||
    !natural(goal.turns_used) ||
    !natural(goal.tokens_used) ||
    !Array.isArray(goal.gaps) ||
    !goal.gaps.every((gap: unknown) => typeof gap === "string") ||
    (goal.active !== undefined && typeof goal.active !== "boolean")
  )
    return null;
  const snoozedUntil = positive(goal.snoozed_until_ms)
    ? goal.snoozed_until_ms
    : null;
  let state: CompactState = GOAL_STATES[goal.status];
  if (goal.status === "active") {
    const waiting = waitSubject(goal.wait);
    if (goal.active === false) {
      state = { stateLabel: "Not owned here (snapshot)", tone: "neutral" };
    } else if (waiting !== null) {
      state = { stateLabel: `${waiting} (snapshot)`, tone: "neutral" };
    } else if (snoozedUntil !== null) {
      state = { stateLabel: "Snooze recorded (snapshot)", tone: "neutral" };
    }
  }
  const budget = budgetText(goal.budget);
  return {
    headline: headline(goal.content) ?? "The current goal",
    ...state,
    facts: compactFacts([
      {
        label: "Latest validation",
        value: `${latestValidation(goal.latest_verdict)} · ${
          goal.gaps.length
        } gaps`,
      },
      snoozedUntil === null
        ? {
            label: "Usage",
            value: `${goal.turns_used} turns · ${goal.tokens_used} tokens`,
          }
        : {
            label: "Recorded snooze until",
            value: `${snoozedUntil} ms since Unix epoch`,
          },
      budget === null
        ? { label: "Base version", value: goal.version }
        : { label: "Budget limits", value: budget },
    ]),
  };
}

function checkpointText(plan: RecordValue): string | null {
  const checkpoint = record(plan.checkpoint);
  if (
    checkpoint === null ||
    !natural(checkpoint.revision) ||
    !natural(checkpoint.folded_deltas) ||
    !natural(plan.delta_count) ||
    checkpoint.folded_deltas > plan.delta_count ||
    checkpoint.approved_version !== plan.version ||
    typeof checkpoint.approved_message_id !== "string" ||
    typeof checkpoint.approved_by !== "string" ||
    !natural(checkpoint.approved_at_ms)
  )
    return null;
  return `Update ${checkpoint.revision} · ${checkpoint.folded_deltas} folded updates`;
}

export function compactGoalPlanPresentation(
  input: CompactAgenticInput,
): CompactAgenticPresentation | null {
  const tool = normalizeToolName(input.toolName);
  if (
    tool === undefined ||
    ![
      "set_plan",
      "update_plan",
      "get_plan",
      "set_goal",
      "update_goal",
      "get_goal",
      "validate_goal",
      "pause_goal",
      "snooze_goal",
    ].includes(tool)
  )
    return null;
  const base: CompactAgenticPresentation = {
    headline: intent(tool, readCompactRecord(input.arguments)),
    ...compactTransportPresentation(input.transportState),
    facts: [],
  };
  if (input.transportState !== "succeeded") return base;
  const result = readCompactRecord(input.result);
  const unconfirmed: CompactAgenticPresentation = {
    ...base,
    stateLabel: "Result unconfirmed",
  };
  switch (tool) {
    case "validate_goal": {
      const verdict = goalEvidenceVerdict(input.extra);
      if (verdict === null)
        return { ...unconfirmed, stateLabel: "Verdict unknown" };
      const receipt = record(record(input.extra)?.validate_goal);
      return {
        ...base,
        ...(verdict === "not_run" && receipt?.verdict === "wait"
          ? { stateLabel: "Waiting; no pass", tone: "neutral" as const }
          : VERDICTS[verdict]),
        facts: compactFacts([
          {
            label: "Evidence scope",
            value:
              verdict === "passed" || verdict === "failed"
                ? "Criteria verdict; executed checks are separate"
                : "No applied criteria verdict",
          },
        ]),
      };
    }
    case "set_goal":
    case "set_plan":
      return positive(result?.version) && result.supersedes === null
        ? {
            ...base,
            stateLabel: tool === "set_goal" ? "Goal accepted" : "Plan accepted",
            facts: compactFacts([
              { label: "Base version", value: result.version },
            ]),
          }
        : unconfirmed;
    case "update_goal":
    case "update_plan":
      return positive(result?.seq) && typeof result.truncated === "boolean"
        ? {
            ...base,
            stateLabel: result.truncated
              ? "Update truncated"
              : "Update accepted",
            tone: result.truncated ? "warning" : "neutral",
            facts: compactFacts([
              { label: "Update sequence", value: result.seq },
            ]),
          }
        : unconfirmed;
    case "get_goal": {
      if (result?.goal === null) return { ...base, stateLabel: "No goal" };
      const goal = record(result?.goal);
      return goal === null
        ? unconfirmed
        : goalPresentation(goal) ?? unconfirmed;
    }
    case "get_plan": {
      if (result?.plan === null) return { ...base, stateLabel: "No plan" };
      const plan = record(result?.plan);
      if (
        plan === null ||
        typeof plan.content !== "string" ||
        typeof plan.mode !== "string" ||
        !positive(plan.version) ||
        !natural(plan.delta_count)
      )
        return unconfirmed;
      const checkpoint = checkpointText(plan);
      return {
        ...base,
        headline: headline(plan.content) ?? base.headline,
        stateLabel: "Plan snapshot",
        facts: compactFacts([
          { label: "Base version", value: plan.version },
          { label: "Updates", value: plan.delta_count },
          checkpoint === null
            ? { label: "Mode", value: humanizeIdentifier(plan.mode) }
            : { label: "Checkpoint", value: checkpoint },
        ]),
      };
    }
    default:
      return unconfirmed;
  }
}
