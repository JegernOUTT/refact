import { useEffect, useId, useRef } from "react";
import { ArrowLeft } from "lucide-react";
import { Button } from "../../components/ui";
import { useGetBuddyOperatorArtifactQuery } from "../../services/refact/buddyOperator";
import type { WorkItem } from "./operatorTypes";
import { BuddyDeskActions } from "./BuddyDeskActions";
import { BuddyReportReadState } from "./BuddyReportReadState";
import { BuddyReportHistory } from "./BuddyReportHistory";
import { BuddyWorkSnooze } from "./BuddyWorkSnooze";
import { BuddyDeskEvidence } from "./BuddyDeskEvidence";
import { BuddyDeskWriteUp } from "./BuddyDeskWriteUp";
import { headline, sameText } from "./buddyUtils";
import {
  BuddyEmptyState,
  BuddyErrorCallout,
  BuddyLoading,
  BuddyStatusBadge,
  BuddySurface,
  BuddyTimestamp,
  toBuddySeverity,
  toBuddyWorkState,
} from "./ui";
import styles from "./BuddyDeskDetail.module.css";

const opaqueIdentifier = /[0-9a-f]{24,}/i;
const uuidLikeIdentifier = /^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/i;

function readableSubject(item: WorkItem): string | null {
  const subject = item.subject.resource_id.trim();
  const machineToken = /^[a-z0-9_-]+$/.test(subject) && /[_-]/.test(subject);
  if (
    subject === "" ||
    item.subject.resource_kind === "workflow" ||
    opaqueIdentifier.test(subject) ||
    uuidLikeIdentifier.test(subject) ||
    machineToken
  )
    return null;
  return subject;
}

function titleRemainder(title: string, body?: string): string | null {
  const lines = title.split(/\r?\n/);
  const headlineIndex = lines.findIndex((line) => line.trim() !== "");
  if (headlineIndex < 0) return null;
  const remainder = lines
    .slice(headlineIndex + 1)
    .join(" ")
    .trim();
  if (!remainder) return null;
  const normalize = (value: string) =>
    value.toLowerCase().replace(/\s+/g, " ").trim();
  return body && normalize(body).includes(normalize(remainder))
    ? null
    : remainder;
}

export function BuddyDeskDetail({
  item,
  endpointIdentity,
  onBack,
  onSnoozeChange,
  onDismiss,
  onDismissPending,
  onDismissFailure,
}: {
  item: WorkItem;
  endpointIdentity: string;
  onBack: () => void;
  onSnoozeChange: () => void;
  onDismiss: (item: WorkItem) => void;
  onDismissPending: (item: WorkItem) => void;
  onDismissFailure: (item: WorkItem) => void;
}) {
  const { currentData, isFetching, error, refetch } =
    useGetBuddyOperatorArtifactQuery(
      {
        id: item.current_artifact_id ?? "",
        revision: item.current_artifact_revision ?? undefined,
        project_id: item.project.project_id,
        endpoint_identity: endpointIdentity,
      },
      { skip: !item.current_artifact_id },
    );
  const titleId = useId();
  const artifact = currentData?.data.value;
  const artifactMatches =
    artifact?.project.project_id === item.project.project_id &&
    artifact.project.canonical_root === item.project.canonical_root &&
    artifact.artifact_id === item.current_artifact_id &&
    (item.current_artifact_revision === null ||
      artifact.revision === item.current_artifact_revision);
  const sameProject =
    artifact?.project.project_id === item.project.project_id &&
    artifact.project.canonical_root === item.project.canonical_root;
  const title = artifact ? headline(artifact.title) : headline(item.title);
  const showConclusion =
    Boolean(artifact?.conclusion.trim()) &&
    !sameText(artifact?.conclusion ?? "", title);
  const backButton = useRef<HTMLButtonElement>(null);
  const subject = readableSubject(item);
  const remainder = artifact
    ? titleRemainder(artifact.title, artifact.body)
    : titleRemainder(item.title);
  const workState = toBuddyWorkState(item.state) ?? "watching";
  const severity = toBuddySeverity(item.severity);
  useEffect(() => {
    backButton.current?.focus({ preventScroll: true });
  }, []);
  return (
    <article className={styles.detail} aria-labelledby={titleId}>
      <header className={styles.header}>
        <Button
          ref={backButton}
          className={styles.back}
          variant="ghost"
          size="sm"
          leftIcon={ArrowLeft}
          onClick={onBack}
        >
          Back to reports
        </Button>
        <h3 id={titleId} className={styles.title}>
          {title || "Untitled report"}
        </h3>
        <div className={styles.headlineMeta}>
          <BuddyStatusBadge dimension="work" value={workState} />
          {severity && (
            <BuddyStatusBadge dimension="severity" value={severity} />
          )}
          {subject && <span className={styles.subject}>{subject}</span>}
          <BuddyTimestamp
            value={item.last_material_change_at}
            prefix="Changed"
          />
        </div>
      </header>
      {!item.current_artifact_id && remainder && (
        <p className={styles.text}>{remainder}</p>
      )}
      {!item.current_artifact_id && (
        <BuddyEmptyState
          compact
          title={`${title} hasn't produced a result yet.`}
        />
      )}
      {isFetching && (
        <BuddyLoading variant="cards" count={1} label="Loading the report" />
      )}
      {error && (
        <BuddyErrorCallout
          thing="this report"
          error={error}
          onRetry={() => void refetch()}
          retryLabel="Retry report"
        />
      )}
      {artifact && !artifactMatches && (
        <BuddyErrorCallout
          thing="this report"
          title={
            sameProject
              ? "This result doesn't match the selected report revision, so nothing from it is shown."
              : "This result belongs to another project."
          }
        />
      )}
      {artifact && artifactMatches && !error && (
        <>
          {(showConclusion || artifact.delta) && (
            <section aria-label="Finding" className={styles.finding}>
              {showConclusion && (
                <p className={styles.conclusion}>{artifact.conclusion}</p>
              )}
              {artifact.delta && (
                <p className={styles.delta}>{artifact.delta}</p>
              )}
            </section>
          )}
          <BuddyDeskWriteUp
            artifact={artifact}
            title={title}
            remainder={remainder}
          />
          <BuddyDeskEvidence
            artifact={artifact}
            persistKey="buddy.reports.detail.evidence"
          />
          <BuddySurface
            as="section"
            level="inset"
            className={styles.manage}
            aria-label="Manage this report"
          >
            <h4 className={styles.label}>Manage this report</h4>
            <BuddyDeskActions
              item={item}
              artifact={artifact}
              endpointIdentity={endpointIdentity}
              onDismiss={onDismiss}
              onDismissPending={onDismissPending}
              onDismissFailure={onDismissFailure}
              secondary={
                <>
                  <BuddyReportReadState
                    key={`read:${artifact.artifact_id}:${artifact.revision}`}
                    item={item}
                    artifact={artifact}
                    endpointIdentity={endpointIdentity}
                  />
                  <BuddyWorkSnooze
                    key={`snooze:${artifact.artifact_id}:${artifact.revision}`}
                    item={item}
                    artifact={artifact}
                    identity={endpointIdentity}
                    onChange={onSnoozeChange}
                  />
                </>
              }
            />
          </BuddySurface>
          <BuddyReportHistory
            item={item}
            artifact={artifact}
            identity={endpointIdentity}
          />
        </>
      )}
    </article>
  );
}
