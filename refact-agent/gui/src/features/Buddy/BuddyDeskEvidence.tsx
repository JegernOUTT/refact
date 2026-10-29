import type { ArtifactRevision } from "./operatorTypes";
import {
  BUDDY_TERMS,
  BuddyDisclosure,
  BuddyEmptyState,
  BuddyTimestamp,
  buddyWorkflowLabel,
  humanizeBuddyIdentifier,
  humanizeBuddyKey,
  distinctBuddyLabels,
} from "./ui";
import styles from "./BuddyDeskDetail.module.css";

const WORKFLOW_KEY = /buddy_[a-z]+(?:_[a-z]+)*/;

function evidenceLabel(locator: string, kind: string): string {
  const workflow = WORKFLOW_KEY.exec(locator)?.[0];
  if (workflow)
    return `${buddyWorkflowLabel(workflow)} ${humanizeBuddyKey(
      kind,
    ).toLowerCase()}`;
  return humanizeBuddyIdentifier(locator).label;
}

type EvidenceGroup = {
  kind: string;
  items: ArtifactRevision["evidence"];
  labels: string[];
};

function groupEvidence(
  evidence: ArtifactRevision["evidence"],
): EvidenceGroup[] {
  const groups = new Map<string, EvidenceGroup>();
  for (const item of evidence) {
    const kind = item.source.source_kind;
    const group = groups.get(kind);
    if (group) group.items.push(item);
    else groups.set(kind, { kind, items: [item], labels: [] });
  }
  return [...groups.values()].map((group) => ({
    ...group,
    labels: distinctBuddyLabels(
      group.items.map((item) => evidenceLabel(item.locator, group.kind)),
    ),
  }));
}

export function BuddyDeskEvidence({
  artifact,
  persistKey,
}: {
  artifact: ArtifactRevision;
  persistKey: string;
}) {
  const groups = groupEvidence(artifact.evidence);
  return (
    <BuddyDisclosure
      label="Evidence"
      count={artifact.evidence.length}
      persistKey={persistKey}
      lazy
      testId="buddy-desk-evidence"
    >
      <p className={styles.hint}>
        Evidence shows what was checked. Verifying it confirms the saved items
        are intact; it doesn&apos;t prove the finding is correct.
      </p>
      {groups.length === 0 ? (
        <BuddyEmptyState compact title="No evidence was recorded." />
      ) : (
        groups.map((group) => (
          <section
            key={group.kind}
            className={styles.evidenceGroup}
            aria-label={humanizeBuddyKey(group.kind)}
          >
            <h4 className={styles.label}>
              {humanizeBuddyKey(group.kind)} · {group.items.length}
            </h4>
            <ul className={styles.evidence}>
              {group.items.map((evidence, index) => {
                return (
                  <li key={evidence.evidence_id} className={styles.evidenceRow}>
                    <span className={styles.evidenceName}>
                      {group.labels[index]}
                    </span>
                    <BuddyTimestamp
                      value={evidence.observed_at}
                      prefix="Observed"
                    />
                  </li>
                );
              })}
            </ul>
          </section>
        ))
      )}
      {artifact.evidence.length > 0 && (
        <BuddyDisclosure label={BUDDY_TERMS.technicalDetails} lazy>
          <pre className={styles.technical}>
            {[
              `Revision ${artifact.revision}`,
              ...artifact.evidence.map(
                (evidence) =>
                  `${evidence.source.source_kind}: ${evidence.locator}`,
              ),
            ].join("\n")}
          </pre>
        </BuddyDisclosure>
      )}
    </BuddyDisclosure>
  );
}
