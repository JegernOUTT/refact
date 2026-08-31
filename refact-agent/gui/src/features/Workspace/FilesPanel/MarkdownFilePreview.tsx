import React, { type MouseEvent } from "react";

import { Markdown } from "../../../components/Markdown";
import { isInteractiveTarget } from "./markdownPreviewTargets";
import styles from "./MarkdownFilePreview.module.css";

export type MarkdownFilePreviewProps = {
  content: string;
  onRequestEdit?: () => void;
};

const MarkdownFilePreviewComponent = ({
  content,
  onRequestEdit,
}: MarkdownFilePreviewProps) => {
  const handleDoubleClick = (event: MouseEvent<HTMLDivElement>) => {
    if (isInteractiveTarget(event.target, event.currentTarget)) return;
    onRequestEdit?.();
  };

  return (
    <div
      aria-label="Markdown preview"
      className={styles.preview}
      data-markdown-preview="true"
      onDoubleClick={handleDoubleClick}
    >
      <div className={styles.content}>
        <Markdown canHaveInteractiveElements={false}>{content}</Markdown>
      </div>
    </div>
  );
};

export const MarkdownFilePreview = React.memo(MarkdownFilePreviewComponent);
