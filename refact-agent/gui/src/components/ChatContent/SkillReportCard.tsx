import React, { useCallback, useState } from "react";
import { Flex, Text, Box } from "@radix-ui/themes";
import {
  LightningBoltIcon,
  CopyIcon,
  CheckIcon,
  FileTextIcon,
} from "@radix-ui/react-icons";
import classNames from "classnames";
import { Markdown, ShikiCodeBlock } from "../Markdown";
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
    true,
  );

  const showSaveButton = isIdeHost();
  const shouldRenderMarkdown =
    report.length <= MAX_MD_RENDER_CHARS && looksLikeMarkdown(report);

  return (
    <div className={classNames(styles.card, styles.variantSkillReport)}>
      <Flex
        className={styles.header}
        align="center"
        gap="2"
        onClick={handleToggle}
      >
        <span className={styles.icon}>
          <LightningBoltIcon />
        </span>
        <Text size="1" className={styles.summary}>
          Skill report: {skillName}
        </Text>
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
              {copied ? <CheckIcon /> : <CopyIcon />}
            </button>
            {showSaveButton && (
              <button
                className={styles.actionButton}
                onClick={handleSave}
                title="Save as file"
              >
                <FileTextIcon />
              </button>
            )}
          </span>
        )}
      </Flex>

      {shouldRender && report && (
        <div
          className={classNames(
            styles.contentWrapper,
            isAnimatingOpen && styles.contentWrapperOpen,
          )}
        >
          <div className={styles.contentInner}>
            <Box className={styles.content}>
              {shouldRenderMarkdown ? (
                <Text size="2">
                  <Markdown>{report}</Markdown>
                </Text>
              ) : (
                <ShikiCodeBlock showLineNumbers={false}>
                  {report}
                </ShikiCodeBlock>
              )}
            </Box>
          </div>
        </div>
      )}
    </div>
  );
};

export default SkillReportCard;
