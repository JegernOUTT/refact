import { useEffect, useRef } from "react";
import { ArrowLeft } from "lucide-react";
import { Button, Surface, Text } from "../../components/ui";
import { useGetBuddyOperatorArtifactQuery } from "../../services/refact/buddyOperator";
import type { WorkItem } from "./operatorTypes";
import { humanizeIdentifier } from "../../utils/displayNames";
import { BuddyDeskActions } from "./BuddyDeskActions";
import { BuddyReportReadState } from "./BuddyReportReadState";
import { BuddyArtifactBody } from "./BuddyArtifactBody";
import { BuddyReportHistory } from "./BuddyReportHistory";
import { BuddyWorkSnooze } from "./BuddyWorkSnooze";
import { headline, sameText } from "./buddyUtils";
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

function observedAgo(value: string): string | null {
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed)) return null;
  const minutes = Math.round((Date.now() - parsed) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}

function observedAbsolute(value: string): string | null {
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed)) return null;
  return new Date(parsed).toLocaleString();
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
  const artifactHeadline = artifact ? headline(artifact.title) : "";
  const showConclusion =
    Boolean(artifact?.conclusion.trim()) &&
    !sameText(artifact?.conclusion ?? "", artifactHeadline);
  let artifactBody: string | null = artifact?.body.trim() ?? null;
  if (artifactBody === "") artifactBody = null;
  if (
    artifactBody &&
    (sameText(artifactBody, artifactHeadline) ||
      sameText(artifactBody, artifact?.conclusion ?? ""))
  ) {
    artifactBody = null;
  } else if (artifactBody) {
    const paragraphBreak = /\r?\n[ \t]*\r?\n/.exec(artifactBody);
    const firstParagraph = paragraphBreak
      ? artifactBody.slice(0, paragraphBreak.index)
      : artifactBody;
    if (sameText(firstParagraph, artifactHeadline)) {
      artifactBody = paragraphBreak
        ? artifactBody
            .slice(paragraphBreak.index + paragraphBreak[0].length)
            .trim() || null
        : null;
    }
    if (
      artifactBody &&
      (sameText(artifactBody, artifactHeadline) ||
        sameText(artifactBody, artifact?.conclusion ?? ""))
    ) {
      artifactBody = null;
    }
  }
  const backButton = useRef<HTMLButtonElement>(null);
  const subject = readableSubject(item);
  const remainder = artifact
    ? titleRemainder(artifact.title, artifact.body)
    : titleRemainder(item.title);
  useEffect(() => {
    backButton.current?.focus();
  }, []);
  return (
    <article className={styles.detail} aria-label="Work detail">
      <Button
        ref={backButton}
        className={styles.back}
        variant="ghost"
        size="sm"
        leftIcon={ArrowLeft}
        onClick={onBack}
      >
        Back to work list
      </Button>
      <div className={styles.headlineMeta}>
        <Text size="1" color="gray" className={styles.subject}>
          {subject
            ? `${humanizeIdentifier(item.state)} · ${subject}`
            : humanizeIdentifier(item.state)}
        </Text>
      </div>
      {!item.current_artifact_id && remainder && (
        <Text as="p" size="2" color="gray">
          {remainder}
        </Text>
      )}
      {!item.current_artifact_id && (
        <Surface variant="surface-1" className={styles.note} role="status">
          <Text size="2" color="gray">
            {headline(item.title)} has not produced a result yet.
          </Text>
        </Surface>
      )}
      {isFetching && (
        <Surface variant="surface-1" className={styles.note} role="status">
          <Text size="2" color="gray">
            Loading the result…
          </Text>
        </Surface>
      )}
      {error && (
        <Surface variant="surface-1" className={styles.note} role="alert">
          <Text size="2" color="red">
            The result could not be loaded.
          </Text>
          <Button size="sm" onClick={() => void refetch()}>
            Retry report
          </Button>
        </Surface>
      )}
      {artifact && !artifactMatches && (
        <Surface variant="surface-1" className={styles.note} role="alert">
          <Text size="2" color="red">
            {sameProject
              ? "This result does not match the selected report revision. Nothing from it is shown."
              : "This result belongs to another project."}
          </Text>
        </Surface>
      )}
      {artifact && artifactMatches && !error && (
        <>
          <Surface
            variant="surface-1"
            radius="card"
            className={styles.headline}
            aria-label="Finding"
            role="region"
          >
            <Text as="h3" size="3" weight="bold" className={styles.title}>
              {artifactHeadline}
            </Text>
            {showConclusion && (
              <Text as="p" size="3" className={styles.conclusion}>
                {artifact.conclusion}
              </Text>
            )}
            {artifact.delta && (
              <Text as="p" size="2" color="gray" className={styles.delta}>
                {artifact.delta}
              </Text>
            )}
          </Surface>
          <details className={styles.disclosure} open>
            <summary className={styles.summary}>The full write-up</summary>
            <div className={styles.disclosureBody}>
              {remainder && (
                <Text as="p" size="2" color="gray">
                  {remainder}
                </Text>
              )}
              {artifactBody && <BuddyArtifactBody body={artifactBody} />}
              {artifact.uncertainty && (
                <section aria-label="Limitations" className={styles.limits}>
                  <Text as="p" size="1" color="orange" weight="medium">
                    What this did not cover
                  </Text>
                  <Text as="p" size="2" color="gray">
                    {artifact.uncertainty}
                  </Text>
                </section>
              )}
              {!remainder && !artifactBody && !artifact.uncertainty && (
                <Text as="p" size="2" color="gray">
                  No further write-up was saved for this revision.
                </Text>
              )}
            </div>
          </details>
          <details className={styles.disclosure}>
            <summary className={styles.summary}>
              Evidence · revision {artifact.revision}
            </summary>
            <div className={styles.disclosureBody}>
              <Text as="p" size="1" color="gray">
                Saved evidence shows what Buddy looked at. Checking it confirms
                the saved artifacts are intact; it does not prove the finding is
                correct.
              </Text>
              {artifact.evidence.length === 0 ? (
                <Text as="p" size="2" color="gray">
                  No evidence references were recorded.
                </Text>
              ) : (
                <ul className={styles.evidence}>
                  {artifact.evidence.map((evidence) => (
                    <li key={evidence.evidence_id}>
                      <Text size="2" weight="medium">
                        {humanizeIdentifier(evidence.source.source_kind)}
                      </Text>
                      <Text size="1" color="gray">
                        {evidence.locator}
                      </Text>
                      {observedAgo(evidence.observed_at) && (
                        <Text
                          size="1"
                          color="gray"
                          title={
                            observedAbsolute(evidence.observed_at) ?? undefined
                          }
                        >
                          Observed {observedAgo(evidence.observed_at)}
                        </Text>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </details>
          <section className={styles.manage} aria-label="Manage this report">
            <Text as="p" size="2" weight="medium" className={styles.title}>
              Manage this report
            </Text>
            <BuddyDeskActions
              item={item}
              artifact={artifact}
              endpointIdentity={endpointIdentity}
              onDismiss={onDismiss}
              onDismissPending={onDismissPending}
              onDismissFailure={onDismissFailure}
            />
            <div className={styles.embedded}>
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
            </div>
          </section>
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
