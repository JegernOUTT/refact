import { useId, useState, type ReactNode } from "react";
import {
  Bot,
  ChevronDown,
  ChevronRight,
  FileText,
  Package,
  Search,
  X,
} from "lucide-react";
import classNames from "classnames";

import {
  Badge,
  Button,
  Icon,
  IconButton,
  ToolCard,
  Tooltip,
} from "../../components/ui";
import {
  isReviewConflict,
  reviewErrorMessage,
  useCancelReviewRunMutation,
  useGetReviewBlobQuery,
  contextIntakeDisplaySchema,
  type BlobRef,
  type Evidence,
  type Material,
  type Review,
  type Run,
} from "../../services/refact/review";
import {
  APPLICABILITY_TONE,
  RUN_TONE,
  formatTime,
  isRunActive,
  label,
} from "./labels";
import { newRequestKey, shortOid } from "./model";
import { useReviewReadOnly } from "./readOnly";
import styles from "./Review.module.css";

export function Drawer({
  title,
  open,
  onToggle,
  placement = "dock",
  chrome = "default",
  id,
  children,
}: {
  title: ReactNode;
  open: boolean;
  onToggle: () => void;
  placement?: "inspector" | "dock";
  chrome?: "default" | "external";
  id?: string;
  children: ReactNode;
}) {
  const generatedId = useId();
  const bodyId = id ?? generatedId;
  return (
    <section
      className={styles.drawer}
      data-placement={placement}
      data-open={open}
      data-chrome={chrome}
    >
      {chrome === "default" && (
        <button
          type="button"
          className={styles.drawerToggle}
          aria-expanded={open}
          aria-controls={bodyId}
          onClick={onToggle}
        >
          <Icon icon={open ? ChevronDown : ChevronRight} size="sm" />
          {title}
        </button>
      )}
      <div
        id={bodyId}
        className={styles.drawerBody}
        hidden={!open}
        aria-label={typeof title === "string" ? title : undefined}
      >
        {children}
      </div>
    </section>
  );
}

const BLOB_PREVIEW_CHARS = 20000;

function BlobViewer({
  reviewId,
  blob,
  name,
}: {
  reviewId: string;
  blob: BlobRef;
  name: string;
}) {
  const [open, setOpen] = useState(false);
  const { data, error, isFetching } = useGetReviewBlobQuery(
    { reviewId, digest: blob.digest },
    { skip: !open },
  );
  let body: ReactNode = null;
  if (open) {
    if (isFetching) body = <span className={styles.meta}>Loading…</span>;
    else if (error) {
      body = <p className={styles.errorText}>{reviewErrorMessage(error)}</p>;
    } else if (data?.kind === "binary") {
      body = (
        <span className={styles.meta}>
          Binary content ({data.size} bytes); not shown.
        </span>
      );
    } else if (data?.kind === "text") {
      const truncated = data.text.length > BLOB_PREVIEW_CHARS;
      body = (
        <>
          <pre className={styles.blob}>
            {truncated ? data.text.slice(0, BLOB_PREVIEW_CHARS) : data.text}
          </pre>
          {truncated && (
            <span className={styles.meta}>
              Showing the first {BLOB_PREVIEW_CHARS} of {data.text.length}{" "}
              characters.
            </span>
          )}
        </>
      );
    }
  }
  return (
    <ToolCard
      title={`${open ? "Hide" : "Show"} ${name} (${blob.size} bytes)`}
      icon={FileText}
      density="compact"
      open={open}
      onOpenChange={setOpen}
    >
      {body ?? <div />}
    </ToolCard>
  );
}

const SUMMARY_CLAMP_CHARS = 240;
const SUMMARY_CLAMP_LINES = 4;
const COMMAND_CLAMP_CHARS = 80;

function EvidenceItem({
  reviewId,
  evidence,
  onRevealSource,
}: {
  reviewId: string;
  evidence: Evidence;
  onRevealSource: (evidence: Evidence) => void;
}) {
  const source = evidence.source;
  const [expanded, setExpanded] = useState(false);
  const summaryId = useId();
  const command = source.kind === "executed" ? source.command.join(" ") : "";
  const long =
    evidence.summary.length > SUMMARY_CLAMP_CHARS ||
    evidence.summary.split("\n").length > SUMMARY_CLAMP_LINES ||
    command.length > COMMAND_CLAMP_CHARS;
  return (
    <article
      className={styles.evidenceCard}
      data-kind={source.kind}
      aria-label={`Evidence: ${evidence.summary}`}
    >
      <div className={styles.evidenceType}>
        <Icon icon={Search} size="sm" /> {label(source.kind)}
      </div>
      <div className={styles.detailBody}>
        <div className={styles.objectHead}>
          <Badge tone={APPLICABILITY_TONE[evidence.applicability]} size="xs">
            {label(evidence.applicability)}
          </Badge>
        </div>
        <p
          id={summaryId}
          className={styles.evidenceSummary}
          data-clamped={long && !expanded}
        >
          {evidence.summary}
        </p>
        {long && (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            aria-expanded={expanded}
            aria-controls={summaryId}
            onClick={() => setExpanded((v) => !v)}
          >
            {expanded ? "Show less" : "Show details"}
          </Button>
        )}
        {source.kind === "source" && (
          <div className={styles.row}>
            <span className={classNames(styles.mono, styles.grow)}>
              {source.path}
              {source.anchor &&
                ` · ${label(source.anchor.position.side)} L${
                  source.anchor.position.start_line
                }–${source.anchor.position.end_line}`}
            </span>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={() => onRevealSource(evidence)}
            >
              {source.anchor
                ? "Show range in diff"
                : "Show file in diff (file context only)"}
            </Button>
          </div>
        )}
        {source.kind === "external" && (
          <span className={styles.mono}>{source.url}</span>
        )}
        {source.kind === "executed" && (
          <>
            <code
              className={classNames(styles.mono, !expanded && styles.truncate)}
              title={command}
            >
              $ {command}
            </code>
            <span
              className={classNames(styles.meta, !expanded && styles.truncate)}
              title={source.workdir}
            >
              in {source.workdir} · exit {source.exit_code ?? "unknown"}
            </span>
            {source.stdout && (
              <BlobViewer
                reviewId={reviewId}
                blob={source.stdout}
                name="stdout"
              />
            )}
            {source.stderr && (
              <BlobViewer
                reviewId={reviewId}
                blob={source.stderr}
                name="stderr"
              />
            )}
          </>
        )}
        <span className={styles.meta}>
          {`Candidate ${evidence.candidate_id} · `}
          {evidence.run_id ? `Run ${evidence.run_id} · ` : ""}
          {evidence.material_id ? `Material ${evidence.material_id} · ` : ""}
          {formatTime(evidence.created_at_ms)}
        </span>
      </div>
    </article>
  );
}

const PRIVACY_INPUT_KEYS = new Set(["derived_origin_labels"]);

function materialIdentity(material: Material): string {
  return JSON.stringify([
    material.kind,
    material.candidate_id,
    material.worktree_id,
    material.worktree_root,
    material.content_digest,
    material.artifacts.map((a) => a.digest),
  ]);
}

function distinctMaterials(materials: Material[]): Material[] {
  const byIdentity = new Map<string, Material>();
  for (const material of materials) {
    const key = materialIdentity(material);
    const existing = byIdentity.get(key);
    if (!existing || (!existing.agent_id && material.agent_id)) {
      byIdentity.set(key, material);
    }
  }
  return [...byIdentity.values()];
}

function MaterialItem({
  reviewId,
  material,
}: {
  reviewId: string;
  material: Material;
}) {
  const entries = Object.entries(material.input_digests);
  const inputs = entries.filter(([name]) => !PRIVACY_INPUT_KEYS.has(name));
  const hasPrivacy = entries.length > inputs.length;
  return (
    <ToolCard
      title={material.description}
      icon={Package}
      density="compact"
      aria-label={`Material: ${material.description}`}
    >
      <div className={styles.detailBody}>
        <div className={styles.objectHead}>
          <span className={styles.grow}>{material.description}</span>
          <Badge tone="default" size="xs">
            {label(material.kind)}
          </Badge>
        </div>
        <span className={styles.mono}>
          digest {shortOid(material.content_digest)}
        </span>
        {material.environment && (
          <span className={styles.meta}>
            Environment: {material.environment}
          </span>
        )}
        {material.agent_id && (
          <span className={styles.meta}>Worker: {material.agent_id}</span>
        )}
        {material.worktree_root && (
          <span className={styles.mono}>{material.worktree_root}</span>
        )}
        {inputs.length > 0 && (
          <span className={styles.meta}>
            Inputs:{" "}
            {inputs
              .map(([name, digest]) => `${name}@${shortOid(digest)}`)
              .join(", ")}
          </span>
        )}
        {hasPrivacy && (
          <span className={styles.meta}>Privacy provenance retained.</span>
        )}
        {material.artifacts.map((artifact, index) => (
          <BlobViewer
            key={artifact.digest}
            reviewId={reviewId}
            blob={artifact}
            name={`artifact ${index + 1}`}
          />
        ))}
      </div>
    </ToolCard>
  );
}

function EvidenceGroup({
  title,
  reviewId,
  items,
  onRevealSource,
}: {
  title: string;
  reviewId: string;
  items: Evidence[];
  onRevealSource: (evidence: Evidence) => void;
}) {
  if (items.length === 0) return null;
  return (
    <>
      <h3 className={styles.sectionLabel}>{title}</h3>
      {items.map((e) => (
        <EvidenceItem
          key={e.id}
          reviewId={reviewId}
          evidence={e}
          onRevealSource={onRevealSource}
        />
      ))}
    </>
  );
}

/**
 * Evidence cited by the selected annotation is listed apart from other
 * consulted evidence; execution output and materials come from isolated runs
 * and are never presented as candidate source.
 */
function ContextIntake({
  review,
  candidateId,
}: {
  review: Review;
  candidateId: string;
}) {
  const candidate = review.candidates.find((item) => item.id === candidateId);
  const receipts = [
    {
      title: "Before Review",
      phase: "before_review",
      value: review.context_intake,
    },
    ...review.tour.map((step) => ({
      title: `Before Tour · ${step.current.title} v${step.current.version}`,
      phase: "before_tour",
      value: step.current.context_intake,
    })),
  ];
  const readOnly = useReviewReadOnly();
  return (
    <section aria-label="Context intake">
      <h3 className={styles.sectionLabel}>Context intake</h3>
      {receipts.map(({ title, phase, value }, index) => {
        const parsed = contextIntakeDisplaySchema.safeParse(value);
        if (!parsed.success)
          return (
            <p key={index} className={styles.meta}>
              {title}: intake {value ? "invalid" : "not recorded"} · unverified.
            </p>
          );
        const receipt = parsed.data;
        const current =
          !readOnly &&
          candidate &&
          receipt.phase === phase &&
          receipt.owner_chat_id === review.owner_chat_id &&
          receipt.repository === candidate.repository &&
          receipt.base_oid === candidate.base_oid &&
          receipt.head_oid === candidate.head_oid &&
          (phase === "before_review"
            ? receipt.candidate_id === null ||
              receipt.candidate_id === candidateId
            : receipt.candidate_id === candidateId);
        return (
          <details key={index}>
            <summary>
              {title} ·{" "}
              {current
                ? "Recorded receipt · identity matches"
                : "Stale or mismatched · unverified"}
            </summary>
            <div className={styles.detailBody}>
              <span className={styles.meta}>
                Receipt {receipt.id} · {formatTime(receipt.captured_at_ms)} ·
                immutable base/head {shortOid(receipt.base_oid)}/
                {shortOid(receipt.head_oid)}. Not a live freshness check.
              </span>
              <p>{receipt.summary}</p>
              <span className={styles.meta}>
                {label(receipt.input_reference.kind)} ·{" "}
                {label(receipt.input_reference.status)} · message{" "}
                {receipt.input_reference.message_id ?? "not recorded"} · tool{" "}
                {receipt.input_reference.tool_call_id ?? "not recorded"}
              </span>
              <strong>
                Recorded retrieval references ({receipt.consultations.length})
              </strong>
              <ul>
                {receipt.consultations.map((item, i) => (
                  <li key={i}>
                    {item.tool_name} · {item.categories.join(", ")} · message{" "}
                    {item.message_id} · call {item.tool_call_id} · result{" "}
                    {shortOid(item.result_hash)} ·{" "}
                    {item.observed_at_ms === null
                      ? "freshness unknown"
                      : formatTime(item.observed_at_ms)}
                  </li>
                ))}
              </ul>
              <strong>Declared gaps ({receipt.gaps.length})</strong>
              <ul>
                {receipt.gaps.map((gap, i) => (
                  <li key={i}>
                    {gap.category} · {gap.status}: {gap.reason}
                  </li>
                ))}
              </ul>
              <span className={styles.meta}>
                Privacy provenance retained ({receipt.privacy.length} records);
                raw records not displayed.
              </span>
            </div>
          </details>
        );
      })}
    </section>
  );
}

export function SourcesAndEvidence({
  review,
  candidateId,
  selectedAnnotationId,
  onRevealSource,
}: {
  review: Review;
  candidateId: string;
  selectedAnnotationId: string | null;
  onRevealSource: (evidence: Evidence) => void;
}) {
  const annotation = selectedAnnotationId
    ? review.annotations.find((a) => a.id === selectedAnnotationId)
    : undefined;
  const citedIds = new Set(annotation?.evidence_ids ?? []);
  const cited = review.evidence.filter((e) => citedIds.has(e.id));
  const missingCitations = [...citedIds].filter(
    (id) => !review.evidence.some((e) => e.id === id),
  );
  const consulted = review.evidence.filter(
    (e) => e.candidate_id === candidateId && !citedIds.has(e.id),
  );
  const materials = distinctMaterials(
    review.materials.filter((m) => m.candidate_id === candidateId),
  );
  return (
    <>
      <ContextIntake review={review} candidateId={candidateId} />
      {annotation && (
        <>
          <h3 className={styles.sectionLabel}>Cited by “{annotation.title}”</h3>
          {cited.length === 0 && (
            <p className={styles.meta}>This annotation cites no evidence.</p>
          )}
          {cited.map((e) => (
            <EvidenceItem
              key={e.id}
              reviewId={review.id}
              evidence={e}
              onRevealSource={onRevealSource}
            />
          ))}
          {missingCitations.length > 0 && (
            <p className={styles.meta}>
              {missingCitations.length} cited item(s) are not available.
            </p>
          )}
        </>
      )}
      <EvidenceGroup
        title={annotation ? "Other consulted sources" : "Consulted sources"}
        reviewId={review.id}
        items={consulted.filter((e) => e.source.kind !== "executed")}
        onRevealSource={onRevealSource}
      />
      <EvidenceGroup
        title="Execution"
        reviewId={review.id}
        items={consulted.filter((e) => e.source.kind === "executed")}
        onRevealSource={onRevealSource}
      />
      {materials.length > 0 && (
        <>
          <h3 className={styles.sectionLabel}>Materials</h3>
          {materials.map((m) => (
            <MaterialItem key={m.id} reviewId={review.id} material={m} />
          ))}
        </>
      )}
      {!annotation && consulted.length === 0 && materials.length === 0 && (
        <p className={styles.meta}>No evidence recorded for this candidate.</p>
      )}
    </>
  );
}

function RunItem({ review, run }: { review: Review; run: Run }) {
  const [cancelRun, { isLoading }] = useCancelReviewRunMutation();
  const readOnly = useReviewReadOnly();
  const [error, setError] = useState<string | null>(null);
  const cancel = async () => {
    setError(null);
    const result = await cancelRun({
      reviewId: review.id,
      runId: run.id,
      expected_revision: review.revision,
      request_key: newRequestKey(),
    });
    if ("error" in result) {
      setError(
        isReviewConflict(result.error)
          ? "The review changed; try again."
          : reviewErrorMessage(result.error),
      );
    }
  };
  return (
    <article
      className={styles.evidenceCard}
      aria-label={`Worker: ${run.purpose}`}
    >
      <div className={styles.evidenceType}>
        <Icon icon={Bot} size="sm" /> Worker
      </div>
      <div className={styles.detailBody}>
        <div className={styles.objectHead}>
          <span className={styles.grow}>{run.purpose}</span>
          <Badge tone={RUN_TONE[run.status]} size="xs">
            {label(run.status)}
          </Badge>
          {isRunActive(run.status) && (
            <Tooltip content={`Cancel ${run.purpose}`}>
              <IconButton
                icon={X}
                disabled={readOnly || run.status === "cancel_requested"}
                size="sm"
                loading={isLoading}
                onClick={() => void cancel()}
                aria-label={`Cancel ${run.purpose}`}
              />
            </Tooltip>
          )}
        </div>
        <span className={styles.meta}>
          {run.agent_id}
          {run.model ? ` · ${run.model}` : ""} · started{" "}
          {formatTime(run.started_at_ms)}
          {run.finished_at_ms
            ? ` · finished ${formatTime(run.finished_at_ms)}`
            : ""}
        </span>
        {run.status === "cancel_requested" && (
          <span className={styles.meta}>Stopping…</span>
        )}
        {run.status === "cancelled" && (
          <span className={styles.meta}>
            Cancelled; work in progress is kept.
          </span>
        )}
        {run.error && <p className={styles.errorText}>{run.error}</p>}
        {error && (
          <p role="alert" className={styles.errorText}>
            {error}
          </p>
        )}
      </div>
    </article>
  );
}

export function WorkerActivity({ review }: { review: Review }) {
  const runs = [...review.runs].sort(
    (a, b) => b.started_at_ms - a.started_at_ms,
  );
  const activity = [...review.activity]
    .sort((a, b) => b.at_ms - a.at_ms)
    .slice(0, 50);
  return (
    <>
      {runs.length === 0 && <p className={styles.meta}>No workers yet.</p>}
      {runs.map((run) => (
        <RunItem key={run.id} review={review} run={run} />
      ))}
      {activity.length > 0 && (
        <>
          <h3 className={styles.sectionLabel}>Activity</h3>
          <ul className={styles.navList}>
            {activity.map((a) => (
              <li key={a.id} className={styles.meta}>
                {formatTime(a.at_ms)} · {label(a.actor.kind)} ·{" "}
                {label(a.action)}
              </li>
            ))}
          </ul>
        </>
      )}
    </>
  );
}
