import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Archive, Github, Pencil } from "lucide-react";

import {
  Button,
  Chip,
  Dialog,
  Field,
  FieldText,
  FieldTextarea,
  Icon,
  Switch,
} from "../../components/ui";
import { useOpenUrl } from "../../hooks/useOpenUrl";
import {
  useCreateBugReportBundleMutation,
  type BugReportContext,
} from "../../services/refact/bugReport";
import { chipKeyHandler } from "./chipKeyHandler";
import { CopyButton } from "./CopyButton";
import { buildGithubIssueUrl, BUG_REPORT_REPO } from "./githubUrl";
import { buildReportTemplate } from "./reportTemplate";
import type { AggregatedError } from "./useBugReportSources";
import styles from "./ReportForm.module.css";

const AVAILABLE_LABELS = [
  "type/bug",
  "needs-triage",
  "component/engine",
  "component/gui",
  "component/ide",
  "P1-important",
];

const DEFAULT_LABELS = ["type/bug", "needs-triage"];

function formatSize(bytes: number): string {
  if (bytes >= 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
  if (bytes >= 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }
  return `${bytes} B`;
}

export type ReportFormProps = {
  context?: BugReportContext;
  errors: AggregatedError[];
  webuiLines: string[];
  host: string;
};

export const ReportForm: React.FC<ReportFormProps> = ({
  context,
  errors,
  webuiLines,
  host,
}) => {
  const openUrl = useOpenUrl();
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [labels, setLabels] = useState<string[]>(DEFAULT_LABELS);
  const [destDir, setDestDir] = useState("");
  const [redact, setRedact] = useState(true);
  const [attachWebui, setAttachWebui] = useState(true);
  const [previewOpen, setPreviewOpen] = useState(false);
  const bodyTouchedRef = useRef(false);

  const [createBundle, bundleResult] = useCreateBugReportBundleMutation();

  useEffect(() => {
    if (bodyTouchedRef.current) return;
    setBody(buildReportTemplate({ context, errors, host }));
  }, [context, errors, host]);

  const handleBodyChange = useCallback((value: string) => {
    bodyTouchedRef.current = true;
    setBody(value);
  }, []);

  const toggleLabel = useCallback((label: string) => {
    setLabels((current) =>
      current.includes(label)
        ? current.filter((item) => item !== label)
        : [...current, label],
    );
  }, []);

  const handleZip = useCallback(() => {
    void createBundle({
      dest_dir: destDir.trim() || undefined,
      redact,
      webui_lines: attachWebui ? webuiLines : undefined,
    });
  }, [attachWebui, createBundle, destDir, redact, webuiLines]);

  const githubUrl = useMemo(
    () => buildGithubIssueUrl({ title, labels, body }),
    [title, labels, body],
  );

  const handleOpenGithub = useCallback(() => {
    setPreviewOpen(false);
    openUrl(githubUrl);
  }, [githubUrl, openUrl]);

  const bundlePath = bundleResult.data?.path;

  return (
    <div className={styles.panel}>
      <div className={styles.header}>
        <Icon icon={Pencil} size="sm" />
        <span className={styles.title}>Describe the bug</span>
        <span className={styles.headerSpacer} />
        <CopyButton
          label="Copy report as Markdown"
          text={`${title}\n\n${body}`}
        />
      </div>

      <div className={styles.form}>
        <Field label="Title">
          <FieldText
            onChange={setTitle}
            placeholder="Short summary — e.g. Chat dies with 'Context too large' loop"
            value={title}
          />
        </Field>

        <Field label="Labels">
          <div className={styles.labels}>
            {AVAILABLE_LABELS.map((label) => (
              <Chip
                key={label}
                className={styles.labelChip}
                onClick={() => toggleLabel(label)}
                onKeyDown={chipKeyHandler(() => toggleLabel(label))}
                role="button"
                selected={labels.includes(label)}
                tabIndex={0}
              >
                {label}
              </Chip>
            ))}
          </div>
        </Field>

        <Field label="Description">
          <FieldTextarea
            className={styles.body}
            onChange={handleBodyChange}
            rows={10}
            value={body}
          />
        </Field>

        <div className={styles.options}>
          <Switch
            checked={attachWebui}
            label="Include Web UI captured errors in the bundle"
            onCheckedChange={setAttachWebui}
          />
          <Switch
            checked={redact}
            label="Redact secrets & tokens"
            onCheckedChange={setRedact}
          />
        </div>

        <Field
          helper={
            context ? `Default: ${context.bundle_default_dir}` : undefined
          }
          label="Bundle folder"
        >
          <FieldText
            onChange={setDestDir}
            placeholder={
              context?.bundle_default_dir ?? "~/.cache/refact/bug-reports"
            }
            value={destDir}
          />
        </Field>

        {bundlePath && bundleResult.data && (
          <div className={styles.bundleReady}>
            <Icon icon={Archive} size="sm" tone="success" />
            <span className={styles.bundleInfo}>
              <span className={styles.bundleName}>
                {formatSize(bundleResult.data.size_bytes)} ·{" "}
                {bundleResult.data.files.length} files
              </span>
              <span className={styles.bundlePath} title={bundlePath}>
                {bundlePath}
              </span>
            </span>
            <CopyButton label="Copy bundle path" text={bundlePath} />
          </div>
        )}
        {bundleResult.isError && (
          <div className={styles.bundleError}>
            Could not create the bundle. Check the destination folder and try
            again.
          </div>
        )}

        <div className={styles.actions}>
          <Button
            loading={bundleResult.isLoading}
            onClick={handleZip}
            variant="soft"
          >
            <Icon icon={Archive} size="sm" />
            Zip everything
          </Button>
          <Button onClick={() => setPreviewOpen(true)} variant="primary">
            <Icon icon={Github} size="sm" />
            Create issue
          </Button>
        </div>
        <div className={styles.hint}>
          Opens a prefilled issue on github.com/{BUG_REPORT_REPO} — attach the
          zip manually before submitting.
        </div>
      </div>

      <Dialog onOpenChange={setPreviewOpen} open={previewOpen}>
        <Dialog.Content maxWidth="640px">
          <Dialog.Title>New issue · {BUG_REPORT_REPO}</Dialog.Title>
          <Dialog.Description>
            Preview before sending. GitHub cannot attach files from a link —
            drop the zip bundle into the issue after it opens.
          </Dialog.Description>
          <div className={styles.previewLabels}>
            {labels.map((label) => (
              <Chip key={label} selected>
                {label}
              </Chip>
            ))}
          </div>
          <pre className={styles.preview}>
            {`Title: ${title || "<no title yet>"}\n\n${body}`}
          </pre>
          <div className={styles.previewActions}>
            <Button onClick={() => setPreviewOpen(false)} variant="ghost">
              Cancel
            </Button>
            <Button onClick={handleOpenGithub} variant="primary">
              Open on GitHub
            </Button>
          </div>
        </Dialog.Content>
      </Dialog>
    </div>
  );
};
