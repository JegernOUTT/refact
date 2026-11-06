import { useCallback, useEffect, useMemo, useRef } from "react";

import { wrapArtifactHtml } from "../../../components/Markdown/renderUtils";
import { useAppearance } from "../../../hooks/useAppearance";
import styles from "./FilesPanel.module.css";

const PREVIEW_POLICY =
  "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; base-uri about:; form-action 'none';";

export function HtmlFilePreview({ content }: { content: string }) {
  const frameRef = useRef<HTMLIFrameElement | null>(null);
  const { appearance } = useAppearance();
  const srcDoc = useMemo(() => {
    const document = new DOMParser().parseFromString(
      `<!DOCTYPE html><meta http-equiv="Content-Security-Policy" content="${PREVIEW_POLICY}">${wrapArtifactHtml(
        content,
      )}`,
      "text/html",
    );
    document
      .querySelectorAll('base, meta[http-equiv="refresh" i]')
      .forEach((node) => node.remove());
    const base = document.createElement("base");
    base.href = "about:srcdoc";
    const policy = document.createElement("meta");
    policy.httpEquiv = "Content-Security-Policy";
    policy.content = PREVIEW_POLICY;
    document.head.prepend(policy, base);
    return `<!DOCTYPE html>\n${document.documentElement.outerHTML}`;
  }, [content]);
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
