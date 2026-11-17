import { BuddyErrorCallout, BuddyLoading } from "./ui";
import styles from "./BuddyDesk.module.css";

export function BuddyDeskNotices({
  notice,
  linkedLoading,
  linkedFailure,
  linkedError,
  projectId,
  onRetryLinked,
}: {
  notice: string | null;
  linkedLoading: boolean;
  linkedFailure: boolean;
  linkedError: unknown;
  projectId?: string;
  onRetryLinked: () => void;
}) {
  return (
    <>
      {notice && (
        <p role="status" className={styles.notice}>
          {notice}
        </p>
      )}
      {linkedLoading && (
        <BuddyLoading variant="rows" count={1} label="Opening that report" />
      )}
      {linkedFailure && (
        <BuddyErrorCallout
          thing="that report"
          title={
            projectId
              ? "Couldn't open that report in this project. No other report was opened."
              : "This link doesn't name a project, so the report wasn't opened."
          }
          error={linkedError}
          onRetry={projectId ? onRetryLinked : undefined}
        />
      )}
    </>
  );
}
