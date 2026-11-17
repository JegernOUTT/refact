import React, { useCallback, useEffect, useRef } from "react";

import { useAppearance } from "../../../hooks/useAppearance";
import type { ArtifactPreviewData } from "./useArtifactPreview";
import styles from "./ArtifactLink.module.css";

export type RenderArtifactMarkdown = (content: string) => React.ReactNode;

const HtmlThumbnail: React.FC<{ srcDoc: string; title: string }> = ({
  srcDoc,
  title,
}) => {
  const frameRef = useRef<HTMLIFrameElement | null>(null);
  const { appearance } = useAppearance();
  const postTheme = useCallback(() => {
    frameRef.current?.contentWindow?.postMessage(
      { type: "refact-artifact-theme", theme: appearance },
      "*",
    );
  }, [appearance]);
  useEffect(postTheme, [postTheme]);
  return (
    <div className={styles.visual}>
      <iframe
        className={styles.htmlFrame}
        onLoad={postTheme}
        ref={frameRef}
        referrerPolicy="no-referrer"
        sandbox="allow-scripts"
        srcDoc={srcDoc}
        tabIndex={-1}
        title={`Preview of ${title}`}
      />
    </div>
  );
};

export const ArtifactPreview: React.FC<{
  preview: ArtifactPreviewData;
  title: string;
  renderMarkdown: RenderArtifactMarkdown;
}> = ({ preview, title, renderMarkdown }) => {
  switch (preview.type) {
    case "image":
    case "svg":
      return (
        <div className={styles.visual}>
          <img
            alt=""
            className={styles.image}
            data-testid="artifact-preview-image"
            src={preview.src}
          />
        </div>
      );
    case "html":
      return <HtmlThumbnail srcDoc={preview.srcDoc} title={title} />;
    case "markdown":
      return (
        <div className={styles.excerpt} data-testid="artifact-preview-text">
          {renderMarkdown(preview.content)}
        </div>
      );
    case "table": {
      const [header, ...rows] = preview.rows;
      return (
        <div className={styles.excerpt} data-testid="artifact-preview-table">
          <table className={styles.table}>
            <thead>
              <tr>
                {header.map((cell, index) => (
                  <th key={index}>{cell}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, rowIndex) => (
                <tr key={rowIndex}>
                  {row.map((cell, index) => (
                    <td key={index}>{cell}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }
    case "text":
      return (
        <div className={styles.excerpt} data-testid="artifact-preview-text">
          <pre className={styles.code}>{preview.content}</pre>
        </div>
      );
  }
};
