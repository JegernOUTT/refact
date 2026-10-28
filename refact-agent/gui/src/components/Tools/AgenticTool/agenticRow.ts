import { normalizeToolName } from "../../../utils/toolNameAliases";
import type { TranscriptStatus } from "../../ChatContent/TranscriptItem";
import { lookupAgenticFamily } from "./agenticFamilies";
import { buildCompactAgenticPresentation } from "./agenticToolModel";
import type { CompactAgenticPresentation } from "./compactPresentation";
import { inspectRow, messageRow, overviewRow } from "./agenticRowAgents";
import { goalRow, planRow, resumeRow, workRow } from "./agenticRowGoal";
import {
  applyTransport,
  emptyRow,
  type AgenticRow,
  type AgenticRowInput,
} from "./agenticRowShared";

const FILLER_FACTS = new Set(["Evidence", "Trust", "Scope", "Wait"]);

const TONE_STATUS: Record<
  CompactAgenticPresentation["tone"],
  TranscriptStatus
> = {
  neutral: "done",
  positive: "done",
  active: "running",
  warning: "unknown",
  danger: "failed",
};

const GOAL_TOOLS = [
  "set_goal",
  "update_goal",
  "get_goal",
  "validate_goal",
  "pause_goal",
  "snooze_goal",
];
const PLAN_TOOLS = ["set_plan", "update_plan", "get_plan"];

function fallbackRow(input: AgenticRowInput, name: string): AgenticRow {
  const descriptor = lookupAgenticFamily(name);
  const verb = descriptor?.label ?? name;
  const presentation = buildCompactAgenticPresentation({
    toolName: name,
    arguments: input.args,
    result: input.content ?? undefined,
    extra: input.extra,
    transportState: input.transport,
  });
  if (!presentation) return { ...emptyRow(verb), useFamilyView: true };
  const status = TONE_STATUS[presentation.tone];
  const quiet =
    status === "done" ||
    presentation.stateLabel === "Response received" ||
    presentation.stateLabel === "Outcome unconfirmed";
  const target =
    presentation.headline === verb ||
    presentation.headline.startsWith(`Request ${verb.toLowerCase()}`)
      ? null
      : presentation.headline;
  return {
    ...emptyRow(verb),
    target,
    status: quiet && status !== "failed" ? "done" : status,
    statusLabel: quiet ? null : presentation.stateLabel,
    facts: presentation.facts.filter((fact) => !FILLER_FACTS.has(fact.label)),
    useFamilyView: true,
  };
}

function rowFor(input: AgenticRowInput, name: string): AgenticRow {
  if (name === "agent_message") return messageRow(input);
  if (name === "agents_overview" || name === "check_agents")
    return overviewRow(input);
  if (
    name === "agent_status" ||
    name === "agent_result" ||
    name === "agent_wait"
  )
    return inspectRow(input, name);
  if (GOAL_TOOLS.includes(name)) return goalRow(input, name);
  if (PLAN_TOOLS.includes(name)) return planRow(input, name);
  if (name === "work_overview" || name === "work_inspect")
    return workRow(input, name);
  if (name === "agent_resume" || name === "agent_lifecycle")
    return resumeRow(input, name);
  return fallbackRow(input, name);
}

export function buildAgenticRow(input: AgenticRowInput): AgenticRow {
  const name = normalizeToolName(input.toolName) ?? input.toolName;
  return applyTransport(rowFor(input, name), input.transport, input.content);
}
