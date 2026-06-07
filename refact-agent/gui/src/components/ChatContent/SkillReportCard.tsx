import React, { useCallback, useState } from "react";
import { Check, Copy, FileText, Zap } from "lucide-react";
import classNames from "classnames";
import { Markdown, ShikiCodeBlock } from "../Markdown";
import { Icon } from "../ui";
import { useDelayedUnmount } from "../shared/useDelayedUnmount";
import { useStoredOpen } from "./useStoredOpen";
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
  const [isOpen, handleToggle] = useStoredOpen(storeKey, true);
  const [animateContent, setAnimateContent] = useState(false);

  const handleAnimatedToggle = useCallback(() => {
    setAnimateContent(true);
    handleToggle();
  }, [handleToggle]);

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

  const { shouldRender, isAnimatingOpen } = useDelayedUnmount(
    isOpen && !!report,
    200,
    animateContent,
  );

  const showSaveButton = isIdeHost();
  const shouldRenderMarkdown =
    report.length <= MAX_MD_RENDER_CHARS && looksLikeMarkdown(report);

  return (
    <div className={classNames(styles.card, styles.variantSkillReport)}>
      <div className={styles.header} onClick={handleAnimatedToggle}>
        <span className={styles.icon}>
          <Icon icon={Zap} size="sm" tone="accent" />
        </span>
        <span className={styles.summary}>Skill report: {skillName}</span>
        {report && (
          <span className={styles.actions}>
            <button
              className={classNames(
                styles.actionButton,
                copied && styles.copiedButton,
              )}
              onClick={handleCopy}
              title="Copy report"
            >
              <Icon icon={copied ? Check : Copy} size="sm" tone={copied ? "success" : "muted"} />
            </button>
            {showSaveButton && (
              <button
                className={styles.actionButton}
                onClick={handleSave}
                title="Save as file"
              >
                <Icon icon={FileText} size="sm" tone="muted" />
              </button>
            )}
          </span>
        )}
      </div>

      {shouldRender && report && (
        <div
          className={classNames(
            styles.contentWrapper,
            isAnimatingOpen && styles.contentWrapperOpen,
            !animateContent && styles.noTransition,
          )}
        >
          <div className={styles.contentInner}>
            <div className={styles.content}>
              {shouldRenderMarkdown ? (
                <div className={styles.markdown}>
                  <Markdown>{report}</Markdown>
                </div>
              ) : (
                <ShikiCodeBlock showLineNumbers={false}>
                  {report}
                </ShikiCodeBlock>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

export default SkillReportCard;
