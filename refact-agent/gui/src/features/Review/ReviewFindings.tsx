import type {
  Annotation,
  Review,
  Severity,
} from "../../services/refact/review";
import { AnnotationCard } from "./ReviewObjects";
import styles from "./ReviewRemote.module.css";

const SEVERITY_ORDER: Severity[] = [
  "critical",
  "high",
  "medium",
  "low",
  "info",
];

function settled(annotation: Annotation): boolean {
  return (
    annotation.disposition === "dismissed" ||
    annotation.disposition === "fixed" ||
    annotation.assessment === "refuted"
  );
}

/** Every finding of the review in one place, most severe first. */
export function ReviewFindings({
  review,
  activeObjectId,
  working,
  onShow,
}: {
  review: Review;
  activeObjectId: string | null;
  working: boolean;
  onShow: (objectId: string) => void;
}) {
  const findings = review.annotations
    .filter((annotation) => annotation.kind === "finding")
    .sort(
      (a, b) =>
        SEVERITY_ORDER.indexOf(a.severity) -
          SEVERITY_ORDER.indexOf(b.severity) || a.title.localeCompare(b.title),
    );
  const groups = [
    { title: "Open", items: findings.filter((item) => !settled(item)) },
    { title: "Settled", items: findings.filter(settled) },
    {
      title: "Notes",
      items: review.annotations.filter((item) => item.kind !== "finding"),
    },
  ].filter((group) => group.items.length > 0);
  return (
    <div className={styles.pane}>
      <div className={styles.list}>
        {groups.length === 0 && (
          <p className={styles.empty}>
            {working
              ? "No findings yet. They appear here as the review checks the change."
              : "No findings were recorded for this change."}
          </p>
        )}
        {groups.map((group) => (
          <section
            key={group.title}
            className={styles.group}
            aria-label={`${group.title} findings`}
          >
            <h3 className={styles.groupTitle}>
              {group.title} · {group.items.length}
            </h3>
            {group.items.map((annotation) => (
              <AnnotationCard
                key={annotation.id}
                review={review}
                annotation={annotation}
                active={annotation.id === activeObjectId}
                location
                onFocus={() => onShow(annotation.id)}
              />
            ))}
          </section>
        ))}
      </div>
    </div>
  );
}
