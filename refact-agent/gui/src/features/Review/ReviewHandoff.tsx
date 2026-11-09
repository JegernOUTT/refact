import { useEffect, useRef, useState } from "react";
import { Check, Copy } from "lucide-react";
import { Button, Dialog, IconButton, Tooltip } from "../../components/ui";
import {
  copyReviewMarkdown,
  downloadReviewMarkdown,
  type HandoffExport,
} from "./reviewHandoffClipboard";
import styles from "./ReviewHandoff.module.css";

export type { HandoffExport } from "./reviewHandoffClipboard";

export function ReviewHandoff({
  reviewId,
  revision,
  disabled = false,
  load,
}: {
  reviewId: string;
  revision: number;
  disabled?: boolean;
  load: () => Promise<HandoffExport>;
}) {
  const identity = `${reviewId}:${revision}:${disabled}`;
  const live = useRef(identity);
  live.current = identity;
  const request = useRef(0);
  const field = useRef<HTMLTextAreaElement>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const [manual, setManual] = useState<HandoffExport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showManual, setShowManual] = useState(false);
  const [downloadError, setDownloadError] = useState<string | null>(null);
  useEffect(() => {
    request.current += 1;
    setBusy(false);
    setCopied(false);
    setManual(null);
    setShowManual(false);
    setDownloadError(null);
    setError(null);
    return () => {
      request.current += 1;
    };
  }, [identity]);
  useEffect(() => {
    if (!copied) return;
    const timeout = setTimeout(() => setCopied(false), 2500);
    return () => clearTimeout(timeout);
  }, [copied]);

  const copy = async () => {
    if (disabled || busy) return;
    const token = ++request.current;
    const current = () =>
      live.current === identity && token === request.current;
    setBusy(true);
    setCopied(false);
    setError(null);
    try {
      const data = await load();
      if (!current()) return;
      if (data.review_id !== reviewId || data.revision < revision) {
        throw new Error(
          "The review changed. Refresh the review before copying.",
        );
      }
      const success = await copyReviewMarkdown(data.markdown);
      if (!current()) return;
      if (success) setCopied(true);
      else setManual(data);
    } catch (e) {
      if (current())
        setError(
          e instanceof Error
            ? e.message
            : "Unable to export review. Please retry.",
        );
    } finally {
      if (current()) setBusy(false);
    }
  };

  return (
    <>
      <Tooltip content={copied ? "Copied" : "Copy review as Markdown"}>
        <IconButton
          icon={copied ? Check : Copy}
          aria-label="Copy review as Markdown"
          size="sm"
          loading={busy}
          disabled={disabled || busy}
          onClick={() => void copy()}
        />
      </Tooltip>
      <span className={styles.status} role="status">
        {copied
          ? "Review Markdown copied"
          : busy
            ? "Preparing review Markdown"
            : ""}
      </span>
      {error && (
        <span className={styles.error} role="alert">
          {error}
        </span>
      )}
      <Dialog
        open={manual !== null}
        onOpenChange={(open) => {
          if (!open) {
            setManual(null);
            setShowManual(false);
            setDownloadError(null);
          }
        }}
      >
        <Dialog.Content maxWidth="720px">
          <Dialog.Title>Copy review Markdown</Dialog.Title>
          <Dialog.Description>
            Clipboard access was unavailable. Download the complete Markdown
            snapshot instead. Nothing has been sent to a chat or agent.
          </Dialog.Description>
          <p>
            Manual selection loads the entire Markdown into the page and may
            freeze your browser for large reviews.
          </p>
          {!showManual && (
            <Button onClick={() => setShowManual(true)}>
              Show full Markdown for manual copy
            </Button>
          )}
          {showManual && (
            <textarea
              ref={field}
              className={styles.markdown}
              aria-label="Review Markdown"
              readOnly
              wrap="off"
              value={manual?.markdown ?? ""}
            />
          )}
          {downloadError && <p role="alert">{downloadError}</p>}
          <p>
            Snapshot revision {manual?.revision}
            {manual?.stack_revision != null
              ? ` · stack revision ${manual.stack_revision}`
              : ""}{" "}
            · {manual?.scope}
          </p>
          <Dialog.Footer>
            <Button
              variant="primary"
              onClick={() => {
                if (manual) {
                  try {
                    downloadReviewMarkdown(manual);
                    setDownloadError(null);
                  } catch {
                    setDownloadError(
                      "Download unavailable. Select and copy the Markdown manually.",
                    );
                  }
                }
              }}
            >
              Download .md
            </Button>
            {showManual && (
              <Button
                onClick={() => {
                  field.current?.focus();
                  field.current?.select();
                }}
              >
                Select all
              </Button>
            )}
            <Dialog.Close asChild>
              <Button>Close</Button>
            </Dialog.Close>
          </Dialog.Footer>
        </Dialog.Content>
      </Dialog>
    </>
  );
}
