import { Badge } from "../../components/ui";
import type {
  BoardCard,
  CardVerificationResult,
  TaskBoard,
} from "../../services/refact/tasks";
import { humanizeIdentifier } from "../../utils/displayNames";
import styles from "./CardEvidencePanel.module.css";

function resultLabel(result: CardVerificationResult): string {
  switch (result.outcome) {
    case "passed":
      return result.passed === true &&
        result.exit_code !== null &&
        result.exit_code !== undefined
        ? "Passed"
        : "Unknown — incomplete passing evidence";
    case "command_failed":
      return "Failed";
    case "rejected":
    case "policy_denied":
      return "Blocked — not run";
    case "no_commands":
      return "Not run";
    case "infrastructure_failed":
      return "Unavailable";
    default:
      return "Unknown — legacy or unclassified evidence";
  }
}

function CommandResults({
  results,
  reported = false,
}: {
  results?: CardVerificationResult[];
  reported?: boolean;
}) {
  if (!results?.length)
    return <p>No command results recorded; execution unknown.</p>;
  return (
    <div className={styles.stack}>
      {results.map((result, index) => (
        <article key={index} className={styles.stack}>
          <strong>
            {reported ? "Reported: " : "Verifier: "}
            {resultLabel(result)}
          </strong>
          <pre className={styles.text}>
            {result.command || "No command recorded"}
          </pre>
          <div>
            Exit: {result.exit_code ?? "unknown"} · Boolean result:{" "}
            {result.passed === undefined ? "unknown" : String(result.passed)}
          </div>
          <div>
            {result.from_cache === true
              ? "Cached — replayed, not re-executed"
              : result.from_cache === false
                ? "Not from cache"
                : "Cache provenance unknown"}
            {result.executed_at
              ? ` · Recorded execution time: ${result.executed_at}`
              : " · Execution time unknown"}
          </div>
          {result.output_tail && (
            <details>
              <summary>Full recorded log tail</summary>
              <pre tabIndex={0} className={styles.text}>
                {result.output_tail}
              </pre>
            </details>
          )}
        </article>
      ))}
    </div>
  );
}

export function CardEvidencePanel({
  card,
  board,
}: {
  card: BoardCard;
  board: TaskBoard;
}) {
  const report = card.final_report_structured;
  const verifier = card.verifier_report;
  const outcome = board.card_outcomes?.[card.id];
  const claim = board.card_claims?.[card.id];
  const attestation = board.review_attestations?.[card.id];
  return (
    <section className={styles.stack} aria-label="Card evidence and authority">
      <h3>Evidence and authority</h3>
      <p>
        Board metadata and agent reports are not proof of execution,
        verification, merge, or finalization.
      </p>
      <dl className={styles.metadata}>
        <dt>Board owner</dt>
        <dd>{board.owner_chat_id ?? "Unknown — no owner recorded"}</dd>
        <dt>Board revision</dt>
        <dd>{board.rev}</dd>
        <dt>Assigned reviewers</dt>
        <dd>
          {board.reviewer_chat_ids?.length
            ? board.reviewer_chat_ids.join(", ")
            : "None recorded / unknown on legacy boards"}
        </dd>
        <dt>Review policy</dt>
        <dd>
          {board.review_policy
            ? humanizeIdentifier(board.review_policy)
            : "Unknown"}
        </dd>
        <dt>Claim</dt>
        <dd>
          {claim
            ? `${claim.claimant_chat_id} at ${claim.claimed_at}`
            : "No claim recorded / unknown on legacy boards"}
        </dd>
        <dt>Review attestation</dt>
        <dd>
          {attestation
            ? `${attestation.reviewer_chat_id} at ${attestation.reviewed_at}`
            : "None recorded"}
        </dd>
        <dt>Finalization</dt>
        <dd>
          {outcome?.finalized
            ? `Finalized by ${outcome.finalized.finalized_by} at ${outcome.finalized.finalized_at}`
            : "Not recorded — not inferred from Done"}
        </dd>
      </dl>
      <details>
        <summary>
          Agent-reported structured report (not independently verified)
        </summary>
        {report ? (
          <div className={styles.stack}>
            <Badge tone="muted">
              Reported{" "}
              {report.success === undefined
                ? "result unknown"
                : report.success
                  ? "success"
                  : "failure"}
            </Badge>
            {report.summary && (
              <pre className={styles.text}>{report.summary}</pre>
            )}
            <CommandResults results={report.verification} reported />
            <details>
              <summary>Full structured agent report</summary>
              <pre tabIndex={0} className={styles.text}>
                {JSON.stringify(report, null, 2)}
              </pre>
            </details>
          </div>
        ) : (
          <p>No structured agent report; reported result unknown.</p>
        )}
      </details>
      <details open={Boolean(verifier)}>
        <summary>Verifier report (separate from the agent claim)</summary>
        {verifier ? (
          <div className={styles.stack}>
            <strong>
              Verifier classification:{" "}
              {humanizeIdentifier(verifier.classification ?? "unknown")}
            </strong>
            <p>
              Verifier boolean result:{" "}
              {verifier.passed === undefined
                ? "unknown"
                : String(verifier.passed)}
              . This report does not establish merge or finalization.
            </p>
            <CommandResults results={verifier.command_results} />
            {verifier.concerns?.length ? (
              <pre className={styles.text}>{verifier.concerns.join("\n")}</pre>
            ) : (
              <p>No concerns recorded.</p>
            )}
            {verifier.recommendation && (
              <pre className={styles.text}>{verifier.recommendation}</pre>
            )}
            <p>
              Spec revision / verified HEAD: not included in this board report.
              Do not substitute the card base commit for verified HEAD.
            </p>
          </div>
        ) : (
          <p>No verifier report. Verification unknown; not a pass.</p>
        )}
      </details>
      <details>
        <summary>Outcome fragments and provenance</summary>
        {!outcome ? (
          <p>No outcome fragments recorded; unknown.</p>
        ) : (
          <div className={styles.stack}>
            {outcome.required.map((requirement) => (
              <div key={requirement.fragment}>
                Required {humanizeIdentifier(requirement.fragment)}
                {requirement.requires_evidence
                  ? " (evidence required)"
                  : ""}:{" "}
                {outcome.fragments.some(
                  (fragment) => fragment.name === requirement.fragment,
                )
                  ? "recorded, not necessarily satisfied"
                  : "missing"}
              </div>
            ))}
            {outcome.fragments.map((fragment, index) => (
              <article key={index} className={styles.stack}>
                <strong>
                  {humanizeIdentifier(fragment.name)} —{" "}
                  {humanizeIdentifier(fragment.review)}
                </strong>
                <div>
                  Source run: {fragment.provenance.source_run ?? "unknown"}
                </div>
                <div>
                  Evidence spec IDs:{" "}
                  {fragment.provenance.evidence_spec_ids.join(", ") ||
                    "none recorded"}
                </div>
                <div>
                  Reviewer: {fragment.reviewer_chat_id ?? "unknown"} · Reviewed
                  at: {fragment.reviewed_at ?? "unknown"}
                </div>
                {fragment.review_note && (
                  <pre className={styles.text}>{fragment.review_note}</pre>
                )}
                <pre tabIndex={0} className={styles.text}>
                  {fragment.content || "Empty fragment"}
                </pre>
              </article>
            ))}
          </div>
        )}
      </details>
    </section>
  );
}
