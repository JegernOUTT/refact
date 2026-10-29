import type { ArtifactRevision } from "./operatorTypes";
import { BuddyArtifactBody } from "./BuddyArtifactBody";
import { stripLeadingTitleHeading } from "./BuddyArtifactBodyText";
import { sameText } from "./buddyUtils";
import { BuddyDisclosure, BuddyEmptyState } from "./ui";
import styles from "./BuddyDeskDetail.module.css";

const STANDARD_DISCLAIMERS = [
  "Buddy's assessment; its findings and any effects have not been independently verified.",
  "Buddy has not changed anything and cannot carry out these steps from here.",
  "Findings are not independently verified.",
];

function isStandardDisclaimer(text: string): boolean {
  return STANDARD_DISCLAIMERS.some((value) => sameText(value, text));
}

function writeUpBody(artifact: ArtifactRevision, title: string): string | null {
  let body: string | null = artifact.body.trim();
  if (body === "") return null;
  body = stripLeadingTitleHeading(body, title);
  if (sameText(body, title) || sameText(body, artifact.conclusion)) return null;
  const paragraphBreak = /\r?\n[ \t]*\r?\n/.exec(body);
  const firstParagraph = paragraphBreak
    ? body.slice(0, paragraphBreak.index)
    : body;
  if (sameText(firstParagraph, title)) {
    body = paragraphBreak
      ? body.slice(paragraphBreak.index + paragraphBreak[0].length).trim() ||
        null
      : null;
  }
  if (body && (sameText(body, title) || sameText(body, artifact.conclusion)))
    return null;
  return body;
}

export function BuddyDeskWriteUp({
  artifact,
  title,
  remainder,
}: {
  artifact: ArtifactRevision;
  title: string;
  remainder: string | null;
}) {
  const body = writeUpBody(artifact, title);
  const uncertainty = artifact.uncertainty?.trim() ?? "";
  const standard = uncertainty !== "" && isStandardDisclaimer(uncertainty);
  return (
    <>
      <BuddyDisclosure
        label="Full write-up"
        defaultOpen
        persistKey="buddy.reports.detail.writeup"
        testId="buddy-desk-writeup"
        contentClassName={styles.disclosureBody}
      >
        {remainder && <p className={styles.text}>{remainder}</p>}
        {body && <BuddyArtifactBody body={body} />}
        {uncertainty !== "" && !standard && (
          <section aria-label="Limitations" className={styles.limits}>
            <h4 className={styles.limitsLabel}>What this didn&apos;t cover</h4>
            <p className={styles.text}>{uncertainty}</p>
          </section>
        )}
        {!remainder && !body && (uncertainty === "" || standard) && (
          <BuddyEmptyState
            compact
            title="No further write-up was saved for this revision."
          />
        )}
      </BuddyDisclosure>
      {standard && (
        <BuddyDisclosure label="Limitations" testId="buddy-desk-limitations">
          <section aria-label="Limitations">
            <p className={styles.text}>{uncertainty}</p>
          </section>
        </BuddyDisclosure>
      )}
    </>
  );
}
