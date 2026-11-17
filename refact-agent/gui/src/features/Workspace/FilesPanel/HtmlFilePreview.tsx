import { useCallback, useEffect, useMemo, useRef } from "react";

import { buildHtmlPreviewDocument } from "../../../components/Markdown/ArtifactLink/htmlPreviewDocument";
import { useAppearance } from "../../../hooks/useAppearance";
import styles from "./FilesPanel.module.css";

export function HtmlFilePreview({ content }: { content: string }) {
  const frameRef = useRef<HTMLIFrameElement | null>(null);
  const { appearance } = useAppearance();
  const srcDoc = useMemo(() => buildHtmlPreviewDocument(content), [content]);
  const postTheme = useCallback(() => {
    frameRef.current?.contentWindow?.postMessage(
      { type: "refact-artifact-theme", theme: appearance },
      "*",
    );
  }, [appearance]);

  useEffect(postTheme, [postTheme]);

  return (
    <iframe
      className={styles.htmlPreview}
      onLoad={postTheme}
      ref={frameRef}
      referrerPolicy="no-referrer"
      sandbox="allow-scripts"
      srcDoc={srcDoc}
      title="HTML preview"
    />
  );
}
