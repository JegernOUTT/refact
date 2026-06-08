import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Code, Copy, Download, ExternalLink, Eye, Play } from "lucide-react";
import { Icon, IconButton, Tooltip } from "../ui";
import { ToolCard, type ToolStatus } from "../ChatContent/ToolCard";
import { PreTag } from "./Pre";
import { useAppearance } from "../../hooks/useAppearance";
import { reportBuddyFrontendError } from "../../features/Buddy/reportBuddyFrontendError";
import styles from "./ArtifactBlock.module.css";
import markdownStyles from "./Markdown.module.css";
import classNames from "classnames";

export interface ArtifactBlockProps {
  code: string;
  isStreaming?: boolean;
  onCopyClick?: (str: string) => void;
}

const MAX_IFRAME_HEIGHT = 800;
const RESIZE_DEBOUNCE_MS = 50;
const MIN_HEIGHT_DELTA = 5;
const MIN_MESSAGE_INTERVAL_MS = 50;
const MAX_ERROR_MESSAGE_LENGTH = 500;

function wrapArtifactHtml(userCode: string, isDark: boolean): string {
  const colorScheme = isDark ? "dark" : "light";
  const theme = isDark ? "dark" : "light";

  const injectedStyles = `<style data-refact-artifact>
html:not([data-artifact-styled]) { color-scheme: ${colorScheme}; }
html:not([data-artifact-styled]) body { margin: 8px; font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; background: Canvas; color: CanvasText; }
</style>`;
  const injectedScripts = `<script data-refact-artifact>
(function() {
  var lastH = 0;
  var timer = null;
  function sendHeight() {
    var h = Math.max(
      document.body.scrollHeight,
      document.body.offsetHeight,
      document.documentElement.scrollHeight,
      document.documentElement.offsetHeight
    );
    if (Math.abs(h - lastH) > ${MIN_HEIGHT_DELTA}) {
      lastH = h;
      window.parent.postMessage({ type: 'refact-artifact-resize', height: h }, '*');
    }
  }
  if (typeof ResizeObserver !== 'undefined') {
    new ResizeObserver(function() {
      clearTimeout(timer);
      timer = setTimeout(sendHeight, ${RESIZE_DEBOUNCE_MS});
    }).observe(document.body);
  }
  window.addEventListener('load', sendHeight);
  setTimeout(sendHeight, 100);
  setTimeout(sendHeight, 500);

  window.onerror = function(msg, src, line, col) {
    window.parent.postMessage({
      type: 'refact-artifact-error',
      message: String(msg),
      line: line,
      col: col
    }, '*');
  };
  window.addEventListener('unhandledrejection', function(e) {
    window.parent.postMessage({
      type: 'refact-artifact-error',
      message: 'Unhandled promise rejection: ' + String(e.reason)
    }, '*');
  });
})();
</script>`;

  const trimmed = userCode.trim();
  const isCompleteDocument =
    trimmed.toLowerCase().startsWith("<!doctype") ||
    trimmed.toLowerCase().startsWith("<html");

  if (isCompleteDocument) {
    const bodyCloseIdx = trimmed.toLowerCase().lastIndexOf("</body>");
    if (bodyCloseIdx !== -1) {
      return (
        trimmed.slice(0, bodyCloseIdx) +
        injectedStyles +
        injectedScripts +
        trimmed.slice(bodyCloseIdx)
      );
    }
    const htmlCloseIdx = trimmed.toLowerCase().lastIndexOf("</html>");
    if (htmlCloseIdx !== -1) {
      return (
        trimmed.slice(0, htmlCloseIdx) +
        injectedStyles +
        injectedScripts +
        trimmed.slice(htmlCloseIdx)
      );
    }
    return trimmed + injectedStyles + injectedScripts;
  }

  return `<!DOCTYPE html>
<html data-theme="${theme}">
<head><meta charset="utf-8">${injectedStyles}</head>
<body>
${userCode}
${injectedScripts}
</body>
</html>`;
}

const _ArtifactBlock: React.FC<ArtifactBlockProps> = ({
  code,
  isStreaming = false,
  onCopyClick,
}) => {
  const [showSource, setShowSource] = useState(false);
  const [isOpen, setIsOpen] = useState(true);
  const [height, setHeight] = useState(200);
  const [error, setError] = useState<string | null>(null);
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const prevStreaming = useRef(isStreaming);
  const { appearance } = useAppearance();
  const isDark = appearance === "dark";

  useEffect(() => {
    if (prevStreaming.current && !isStreaming) {
      setShowSource(false);
    }
    prevStreaming.current = isStreaming;
  }, [isStreaming]);

  const wrappedHtml = useMemo(
    () => wrapArtifactHtml(code, isDark),
    [code, isDark],
  );

  useEffect(() => {
    let lastMessageTime = 0;
    const handler = (event: MessageEvent) => {
      if (event.source !== iframeRef.current?.contentWindow) return;
      const data = event.data as Record<string, unknown> | null;
      if (!data || typeof data.type !== "string") return;

      const now = Date.now();
      if (now - lastMessageTime < MIN_MESSAGE_INTERVAL_MS) return;
      lastMessageTime = now;

      if (data.type === "refact-artifact-resize") {
        const h = Number(data.height);
        if (h > 0) {
          setHeight(Math.min(h, MAX_IFRAME_HEIGHT));
        }
      }
      if (data.type === "refact-artifact-error") {
        const msg = String(data.message).slice(0, MAX_ERROR_MESSAGE_LENGTH);
        setError(msg);
        void reportBuddyFrontendError({
          source: "artifact_iframe",
          error: msg,
          sourceFile: "frontend/artifact_iframe",
          toolName: "artifact_iframe",
        });
      }
    };
    window.addEventListener("message", handler);
    return () => window.removeEventListener("message", handler);
  }, []);

  useEffect(() => {
    setError(null);
  }, [code]);

  const handleToggle = useCallback(() => setIsOpen((v) => !v), []);
  const handleToggleSource = useCallback(() => setShowSource((v) => !v), []);

  const handleCopy = useCallback(() => {
    onCopyClick?.(code);
  }, [onCopyClick, code]);

  const handleOpenInTab = useCallback(() => {
    const wrapperHtml = `<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>HTML Preview</title>
<style>*{margin:0;padding:0}html,body{height:100%}iframe{width:100%;height:100vh;border:0}</style>
</head><body>
<iframe id="f" sandbox="allow-scripts" referrerpolicy="no-referrer"></iframe>
<script>document.getElementById('f').srcdoc=${JSON.stringify(
      wrappedHtml,
    )};</script>
</body></html>`;
    const blob = new Blob([wrapperHtml], { type: "text/html" });
    const url = URL.createObjectURL(blob);
    window.open(url, "_blank", "noopener,noreferrer");
    setTimeout(() => URL.revokeObjectURL(url), 60000);
  }, [wrappedHtml]);

  const handleDownload = useCallback(() => {
    const blob = new Blob([wrappedHtml], { type: "text/html" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "artifact.html";
    a.click();
    URL.revokeObjectURL(url);
  }, [wrappedHtml]);

  const lineCount = useMemo(() => code.split("\n").length, [code]);

  const status: ToolStatus = useMemo(() => {
    if (isStreaming) return "running";
    if (error) return "error";
    return "success";
  }, [isStreaming, error]);

  const effectiveShowSource = isStreaming || showSource;
  const showIframe = !isStreaming && !showSource;

  return (
    <ToolCard
      icon={<Icon icon={Play} size="sm" />}
      summary="HTML Preview"
      meta={`${lineCount} lines`}
      status={status}
      isOpen={isOpen}
      onToggle={handleToggle}
    >
      <div className={styles.artifact_container}>
        <div className={styles.tab_bar}>
          <Tooltip>
            <Tooltip.Trigger asChild>
              <IconButton
                size="sm"
                variant="ghost"
                onClick={handleToggleSource}
                disabled={isStreaming}
                aria-label={showSource ? "Show preview" : "Show source"}
                icon={showSource ? Eye : Code}
              />
            </Tooltip.Trigger>
            <Tooltip.Content>
              {showSource ? "Show preview" : "Show source"}
            </Tooltip.Content>
          </Tooltip>
          <div className={styles.tab_bar_spacer} />
          <Tooltip>
            <Tooltip.Trigger asChild>
              <IconButton
                size="sm"
                variant="ghost"
                onClick={handleOpenInTab}
                disabled={isStreaming}
                aria-label="Open in new tab"
                icon={ExternalLink}
              />
            </Tooltip.Trigger>
            <Tooltip.Content>Open in new tab</Tooltip.Content>
          </Tooltip>
          <Tooltip>
            <Tooltip.Trigger asChild>
              <IconButton
                size="sm"
                variant="ghost"
                onClick={handleDownload}
                disabled={isStreaming}
                aria-label="Download HTML file"
                icon={Download}
              />
            </Tooltip.Trigger>
            <Tooltip.Content>Download as .html</Tooltip.Content>
          </Tooltip>
          {onCopyClick && (
            <Tooltip>
              <Tooltip.Trigger asChild>
                <IconButton
                  size="sm"
                  variant="ghost"
                  onClick={handleCopy}
                  aria-label="Copy HTML source"
                  icon={Copy}
                />
              </Tooltip.Trigger>
              <Tooltip.Content>Copy source</Tooltip.Content>
            </Tooltip>
          )}
        </div>

        {effectiveShowSource && (
          <div className={styles.source_view}>
            <PreTag className={markdownStyles.shiki_pre}>
              <code
                className={classNames(
                  markdownStyles.code,
                  markdownStyles.code_block,
                )}
              >
                {code}
              </code>
            </PreTag>
          </div>
        )}

        {showIframe && (
          <iframe
            ref={iframeRef}
            className={styles.iframe}
            srcDoc={wrappedHtml}
            sandbox="allow-scripts"
            referrerPolicy="no-referrer"
            title="HTML Preview"
            style={{ height: `${height}px` }}
          />
        )}

        {error && <div className={styles.error_bar}>JS Error: {error}</div>}
      </div>
    </ToolCard>
  );
};

export const ArtifactBlock = React.memo(_ArtifactBlock);
