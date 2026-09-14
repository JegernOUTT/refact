import { useState } from "react";
import { Badge, Button } from "../../ui";
import { normalizeToolName } from "../../../utils/toolNameAliases";
import styles from "./AgenticTool.module.css";

type PlanResult =
  | { kind: "empty" }
  | {
      kind: "snapshot";
      content: string;
      version: number;
      deltas: number;
      mode: string;
    }
  | { kind: "installed"; version: number }
  | { kind: "updated"; seq: number; truncated: boolean };

function natural(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parse(toolName: string, content: string | null): PlanResult | null {
  let value: unknown;
  try {
    value = JSON.parse(content ?? "null");
  } catch {
    return null;
  }
  if (!record(value)) return null;
  switch (normalizeToolName(toolName)) {
    case "get_plan": {
      if (value.plan === null) return { kind: "empty" };
      const plan = value.plan;
      if (
        !record(plan) ||
        typeof plan.content !== "string" ||
        typeof plan.mode !== "string" ||
        !natural(plan.version) ||
        plan.version === 0 ||
        !natural(plan.delta_count)
      )
        return null;
      return {
        kind: "snapshot",
        content: plan.content,
        version: plan.version,
        deltas: plan.delta_count,
        mode: plan.mode,
      };
    }
    case "set_plan":
      return natural(value.version) &&
        value.version > 0 &&
        value.supersedes === null
        ? { kind: "installed", version: value.version }
        : null;
    case "update_plan":
      return natural(value.seq) &&
        value.seq > 0 &&
        typeof value.truncated === "boolean"
        ? { kind: "updated", seq: value.seq, truncated: value.truncated }
        : null;
    default:
      return null;
  }
}

const PAGE_CHARS = 4096;

export function PlanToolView({
  toolName,
  content,
  pending,
  failed,
}: {
  toolName: string;
  content: string | null;
  pending: boolean;
  failed: boolean;
}) {
  const [page, setPage] = useState(0);
  if (pending)
    return (
      <p role="status">
        Waiting for the plan receipt. No plan change confirmed yet.
      </p>
    );
  const result = parse(toolName, content);
  if (failed || result === null)
    return (
      <p role="alert">
        Plan details unavailable. This error or legacy response does not confirm
        a plan change or completed work.
      </p>
    );
  const body = result.kind === "snapshot" ? result.content : "";
  const pages = Math.max(1, Math.ceil(body.length / PAGE_CHARS));
  const index = Math.min(page, pages - 1);
  return (
    <section aria-label="Plan receipt" className={styles.section}>
      <div className={styles.metaRow}>
        <Badge tone="muted">Historical plan receipt</Badge>
        <Badge tone="muted">
          {result.kind === "snapshot"
            ? "Read only"
            : result.kind === "empty"
              ? "No plan"
              : "Change accepted"}
        </Badge>
      </div>
      {result.kind === "empty" ? (
        <p>No plan was installed in this snapshot.</p>
      ) : (
        <dl className={styles.detailGrid}>
          <div className={styles.detailItem}>
            <dt className={styles.detailLabel}>
              {result.kind === "updated" ? "Update sequence" : "Base version"}
            </dt>
            <dd className={styles.detailValue}>
              {result.kind === "updated" ? result.seq : result.version}
            </dd>
          </div>
          {result.kind === "snapshot" && (
            <>
              <div className={styles.detailItem}>
                <dt className={styles.detailLabel}>Mode</dt>
                <dd className={styles.detailValue}>
                  {result.mode.slice(0, 240)}
                </dd>
              </div>
              <div className={styles.detailItem}>
                <dt className={styles.detailLabel}>Updates</dt>
                <dd className={styles.detailValue}>{result.deltas}</dd>
              </div>
            </>
          )}
        </dl>
      )}
      {result.kind === "installed" && (
        <p>
          The base plan was accepted. This is not an execution or verification
          result.
        </p>
      )}
      {result.kind === "updated" && (
        <p>
          {result.truncated
            ? "The update was accepted with truncation. Inspect the current plan before relying on omitted constraints."
            : "The update was accepted without truncation. The approved base remains separate from its updates."}
        </p>
      )}
      {result.kind === "snapshot" && (
        <>
          <div aria-label="Plan content" className={styles.body}>
            {body.slice(index * PAGE_CHARS, (index + 1) * PAGE_CHARS)}
          </div>
          {pages > 1 && (
            <nav aria-label="Plan content pages" className={styles.actions}>
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
        </>
      )}
      <p className={styles.note}>
        A plan describes intent. This receipt does not establish approval to
        execute commands or prove that any step passed verification.
      </p>
    </section>
  );
}
