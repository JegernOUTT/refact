import { useState } from "react";
import { Badge, Button } from "../../ui";
import { normalizeToolName } from "../../../utils/toolNameAliases";
import { humanizeIdentifier } from "../../../utils/displayNames";
import styles from "./AgenticTool.module.css";

type GoalReceipt = {
  label: string;
  explanation: string;
  body: string;
  facts: [string, string | number][];
  gaps?: string[];
};

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function natural(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function parse(
  toolName: string,
  content: string,
  extra: unknown,
): GoalReceipt | null {
  const tool = normalizeToolName(toolName);
  if (
    tool === "pause_goal" &&
    content ===
      "Goal pursuit paused. The user can resume it with goal_control(resume)."
  ) {
    return {
      label: "Pause accepted",
      explanation:
        "Pursuit was paused at this receipt. Only an authorized user can resume it.",
      body: "",
      facts: [],
    };
  }
  if (tool === "snooze_goal") {
    const match =
      /^Goal pursuit snoozed for (\d+) minutes; it resumes automatically afterwards or on the next user message\.$/u.exec(
        content,
      );
    if (!match || Number(match[1]) < 1 || Number(match[1]) > 1440) return null;
    return {
      label: "Snooze accepted",
      explanation:
        "This is a timed hold, not a completed goal. The hold may already have ended.",
      body: "",
      facts: [["Minutes", Number(match[1])]],
    };
  }
  if (tool === "validate_goal") {
    const validation = record(extra) ? extra.validate_goal : null;
    const verdict = record(validation) ? validation.verdict : null;
    if (verdict === "met" || verdict === "unmet") {
      return {
        label: verdict === "met" ? "Goal marked complete" : "Remaining gaps",
        explanation:
          "This records a semantic verifier verdict, not proof that tests, builds or artifact checks ran or passed.",
        body: content,
        facts: [["Verdict", verdict === "met" ? "Met" : "Not yet met"]],
      };
    }
    const label = content.startsWith("Goal validation could not run:")
      ? "Validation unavailable"
      : content === "Goal state changed during validation; verdict discarded."
        ? "Verdict superseded"
        : content.startsWith("Goal budget exhausted — cannot validate;")
          ? "Validation blocked by budget"
          : content === "No active goal to validate."
            ? "No active goal to validate"
            : null;
    return label
      ? {
          label,
          explanation:
            "No applicable success verdict is established by this receipt.",
          body: content,
          facts: [],
        }
      : null;
  }
  let value: unknown;
  try {
    value = JSON.parse(content);
  } catch {
    return null;
  }
  if (!record(value)) return null;
  if (tool === "set_goal") {
    return natural(value.version) &&
      value.version > 0 &&
      value.supersedes === null
      ? {
          label: "Goal accepted",
          explanation: "The base goal was accepted, not completed or verified.",
          body: "",
          facts: [["Base version", value.version]],
        }
      : null;
  }
  if (tool === "update_goal") {
    return natural(value.seq) &&
      value.seq > 0 &&
      typeof value.truncated === "boolean"
      ? {
          label: "Update accepted",
          explanation: value.truncated
            ? "The update was truncated. Inspect the current goal before relying on omitted constraints."
            : "The update was accepted without truncation; completion is not implied.",
          body: "",
          facts: [["Update sequence", value.seq]],
        }
      : null;
  }
  if (tool !== "get_goal") return null;
  if (value.goal === null)
    return {
      label: "No goal",
      explanation: "No goal was installed in this snapshot.",
      body: "",
      facts: [],
    };
  const goal = value.goal;
  if (
    !record(goal) ||
    typeof goal.content !== "string" ||
    typeof goal.status !== "string" ||
    !natural(goal.version) ||
    goal.version === 0 ||
    !natural(goal.delta_count) ||
    !natural(goal.turns_used) ||
    !natural(goal.tokens_used) ||
    !Array.isArray(goal.gaps) ||
    !goal.gaps.every((gap: unknown) => typeof gap === "string")
  )
    return null;
  return {
    label: "Read only",
    explanation:
      "Status and usage are historical observations, not live pursuit controls. Missing budget limits are not zero limits.",
    body: goal.content,
    gaps: goal.gaps,
    facts: [
      ["Status", humanizeIdentifier(goal.status.slice(0, 120))],
      ["Base version", goal.version],
      ["Updates", goal.delta_count],
      ["Turns used", goal.turns_used],
      ["Tokens used", goal.tokens_used],
      ["Remaining gaps", goal.gaps.length],
    ],
  };
}

const PAGE_CHARS = 4096;

function GoalGaps({ gaps }: { gaps: string[] }) {
  const [gap, setGap] = useState(0);
  const [section, setSection] = useState(0);
  const index = Math.min(gap, gaps.length - 1);
  const text = gaps[index] ?? "";
  const pages = Math.max(1, Math.ceil(text.length / PAGE_CHARS));
  const page = Math.min(section, pages - 1);
  return (
    <section aria-label="Goal gaps" className={styles.section}>
      <div className={styles.body}>
        {text.slice(page * PAGE_CHARS, (page + 1) * PAGE_CHARS)}
      </div>
      <nav aria-label="Goal gap pages" className={styles.actions}>
        <Button
          size="sm"
          disabled={index === 0}
          onClick={() => {
            setGap(index - 1);
            setSection(0);
          }}
        >
          Previous gap
        </Button>
        <span role="status">
          Gap {index + 1} of {gaps.length}
        </span>
        <Button
          size="sm"
          disabled={index + 1 === gaps.length}
          onClick={() => {
            setGap(index + 1);
            setSection(0);
          }}
        >
          Next gap
        </Button>
      </nav>
      {pages > 1 && (
        <nav aria-label="Long gap sections" className={styles.actions}>
          <Button
            size="sm"
            disabled={page === 0}
            onClick={() => setSection(page - 1)}
          >
            Previous gap section
          </Button>
          <span role="status">
            Section {page + 1} of {pages}
          </span>
          <Button
            size="sm"
            disabled={page + 1 === pages}
            onClick={() => setSection(page + 1)}
          >
            Next gap section
          </Button>
        </nav>
      )}
    </section>
  );
}

export function GoalToolView({
  toolName,
  content,
  extra,
  pending,
  failed,
}: {
  toolName: string;
  content: string | null;
  extra?: unknown;
  pending: boolean;
  failed: boolean;
}) {
  const [page, setPage] = useState(0);
  if (pending)
    return (
      <p role="status">
        Waiting for the goal receipt. No change or verdict confirmed yet.
      </p>
    );
  if (failed)
    return (
      <p role="alert">
        Goal operation failed. No change or successful verification is
        confirmed.
      </p>
    );
  const result = parse(toolName, content ?? "", extra);
  const body = result?.body ?? content ?? "";
  const pages = Math.max(1, Math.ceil(body.length / PAGE_CHARS));
  const index = Math.min(page, pages - 1);
  return (
    <section aria-label="Goal receipt" className={styles.section}>
      <div className={styles.metaRow}>
        <Badge tone="muted">Historical goal receipt</Badge>
        <Badge tone="muted">
          {result?.label ?? "Legacy or unavailable details"}
        </Badge>
      </div>
      <p role={result ? undefined : "alert"}>
        {result?.explanation ??
          "This unrecognized response does not confirm a change or verification result. Text below is untrusted historical output."}
      </p>
      {result && result.facts.length > 0 && (
        <dl className={styles.detailGrid}>
          {result.facts.map(([label, value]) => (
            <div className={styles.detailItem} key={label}>
              <dt className={styles.detailLabel}>{label}</dt>
              <dd className={styles.detailValue}>{value}</dd>
            </div>
          ))}
        </dl>
      )}
      {body.length > 0 && (
        <div aria-label="Goal content" className={styles.body}>
          {body.slice(index * PAGE_CHARS, (index + 1) * PAGE_CHARS)}
        </div>
      )}
      {pages > 1 && (
        <nav aria-label="Goal content pages" className={styles.actions}>
          <Button
            size="sm"
            disabled={index === 0}
            onClick={() => setPage(index - 1)}
          >
            Previous section
          </Button>
          <span role="status">
            Section {index + 1} of {pages}
          </span>
          <Button
            size="sm"
            disabled={index + 1 === pages}
            onClick={() => setPage(index + 1)}
          >
            Next section
          </Button>
        </nav>
      )}
      {result?.gaps && result.gaps.length > 0 && (
        <GoalGaps gaps={result.gaps} />
      )}
      <p className={styles.note}>
        Intent, reported completion and executed evidence are distinct. This
        card cannot grant permissions or increase budgets.
      </p>
    </section>
  );
}
