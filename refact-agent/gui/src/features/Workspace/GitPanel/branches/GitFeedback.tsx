import { X } from "lucide-react";

import { IconButton } from "../../../../components/ui";
import type { GitErrorView } from "../gitErrors";
import styles from "./Branches.module.css";

const MAX_LISTED_FILES = 12;

export function GitErrorDetails({ error }: { error: GitErrorView }) {
  const shown = error.files.slice(0, MAX_LISTED_FILES);
  const hidden = error.files.length - shown.length;
  return (
    <>
      <span>{error.message}</span>
      {shown.length > 0 ? (
        <ul aria-label="Conflicting files" className={styles.fileList}>
          {shown.map((file) => (
            <li key={file}>{file}</li>
          ))}
          {hidden > 0 ? <li>+{hidden} more</li> : null}
        </ul>
      ) : null}
      {error.output ? (
        <details className={styles.rawOutput}>
          <summary>Git output</summary>
          <pre className="scrollX">{error.output}</pre>
        </details>
      ) : null}
    </>
  );
}

export type GitFeedbackState =
  | { tone: "success"; text: string }
  | { tone: "error"; error: GitErrorView };

type GitFeedbackProps = {
  feedback: GitFeedbackState | null;
  onDismiss: () => void;
};

export function GitFeedback({ feedback, onDismiss }: GitFeedbackProps) {
  if (!feedback) return null;
  const success = feedback.tone === "success";
  return (
    <div
      className={styles.feedback}
      data-tone={feedback.tone}
      role={success ? "status" : "alert"}
    >
      <div className={styles.feedbackBody}>
        {success ? (
          <span>{feedback.text}</span>
        ) : (
          <GitErrorDetails error={feedback.error} />
        )}
      </div>
      <IconButton
        aria-label="Dismiss"
        icon={X}
        onClick={onDismiss}
        size="sm"
        variant="ghost"
      />
    </div>
  );
}
