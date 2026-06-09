import React, { useCallback, useState } from "react";
import { Check, Copy, FileText, Zap } from "lucide-react";
import classNames from "classnames";
import { Markdown, ShikiCodeBlock } from "../Markdown";
import { Icon } from "../ui";
import { useStoredOpen } from "./useStoredOpen";
import { AnimatedCollapsible } from "./shared/AnimatedCollapsible";
import { useCopyToClipboard } from "../../hooks/useCopyToClipboard";
import { useEventsBusForIDE } from "../../hooks";
import { isIdeHost } from "../../utils/isIdeHost";
import styles from "./SkillReportCard.module.css";

const MAX_MD_RENDER_CHARS = 50_000;

function looksLikeMarkdown(text: string): boolean {
  if (text.includes("```")) return true;
  if (/\[[^\]]+\]\([^)]+\)/.test(text)) return true;
  if (/^#{1,6}\s+\S/m.test(text)) return true;
  if (/^\s*([-*+])\s+\S/m.test(text)) return true;
  if (/^\s*\d+\.\s+\S/m.test(text)) return true;
  const hasTableHeader = /^\s*\|.+\|\s*$/m.test(text);
  const hasTableSep = /^\s*\|[\s:|-]+\|\s*$/m.test(text);
  if (hasTableHeader && hasTableSep) return true;
  return false;
}

interface SkillReportCardProps {
  skillName: string;
  report: string;
  storeKey: string;
}

export const SkillReportCard: React.FC<SkillReportCardProps> = ({
  skillName,
  report,
  storeKey,
}) => {
  const copyToClipboard = useCopyToClipboard();
  const { newFile } = useEventsBusForIDE();
  const [copied, setCopied] = useState(false);
  const [isOpen, , setIsOpen] = useStoredOpen(storeKey, true);

  const handleCopy = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      if (report) {
        copyToClipboard(report);
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      }
    },
    [report, copyToClipboard],
  );

  const handleSave = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      if (report) {
        newFile(report);
      }
    },
    [report, newFile],
  );


  const showSaveButton = isIdeHost();
  const shouldRenderMarkdown =
    report.length <= MAX_MD_RENDER_CHARS && looksLikeMarkdown(report);

  return (
    <AnimatedCollapsible
      actions={
        report ? (
          <span className={styles.actions}>
            <button
              aria-label="Copy report"
              className={classNames(
                styles.actionButton,
                copied && styles.copiedButton,
              )}
              onClick={handleCopy}
              title="Copy report"
              type="button"
            >
              <Icon
                icon={copied ? Check : Copy}
                size="sm"
                tone={copied ? "success" : "muted"}
              />
            </button>
            {showSaveButton && (
              <button
                aria-label="Save as file"
                className={styles.actionButton}
                onClick={handleSave}
                title="Save as file"
                type="button"
              >
                <Icon icon={FileText} size="sm" tone="muted" />
              </button>
            )}
          </span>
        ) : undefined
      }
      className={classNames(styles.card, styles.variantSkillReport)}
      header={<span className={styles.summary}>Skill report: {skillName}</span>}
      icon={<Icon icon={Zap} size="sm" tone="accent" />}
      onOpenChange={setIsOpen}
      open={isOpen}
      status="success"
      variant="compact"
    >
      {report && (
        <div className={styles.content}>
          {shouldRenderMarkdown ? (
            <div className={styles.markdown}>
              <Markdown>{report}</Markdown>
            </div>
          ) : (
            <ShikiCodeBlock showLineNumbers={false}>{report}</ShikiCodeBlock>
          )}
        </div>
      )}
    </AnimatedCollapsible>
  );
};

export default SkillReportCard;
