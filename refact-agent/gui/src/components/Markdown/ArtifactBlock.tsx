import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Code,
  Copy,
  Download,
  ExternalLink,
  Eye,
  FileCode,
  Globe,
  RefreshCw,
} from "lucide-react";
import { Icon, IconButton, Tooltip } from "../ui";
import { ToolCard, type ToolStatus } from "../ChatContent/ToolCard";
import { PreTag } from "./Pre";
import { useAppearance } from "../../hooks/useAppearance";
import { useConfig } from "../../hooks/useConfig";
import { useEventsBusForIDE } from "../../hooks/useEventBusForIDE";
import { reportBuddyFrontendError } from "../../features/Buddy/reportBuddyFrontendError";
import {
  DEFAULT_IFRAME_HEIGHT,
  MAX_IFRAME_HEIGHT,
  MIN_HEIGHT_DELTA,
  wrapArtifactHtml,
} from "./renderUtils";
import styles from "./ArtifactBlock.module.css";
import markdownStyles from "./Markdown.module.css";
import classNames from "classnames";

export interface ArtifactBlockProps {
  code: string;
  isStreaming?: boolean;
  onCopyClick?: (str: string) => void;
}

const MAX_ERROR_MESSAGE_LENGTH = 500;
const MAX_ERROR_REPORTS_PER_MOUNT = 3;
const HEIGHT_CACHE_LIMIT = 50;
const BLOB_URL_REVOKE_DELAY_MS = 60_000;

// Measured iframe heights survive virtualization unmounts so scrolled-back
// previews reappear at their real height instead of jumping from the default.
const iframeHeightCache = new Map<string, number>();

function rememberHeight(key: string, value: number) {
  if (iframeHeightCache.has(key)) iframeHeightCache.delete(key);
  iframeHeightCache.set(key, value);
  if (iframeHeightCache.size > HEIGHT_CACHE_LIMIT) {
    const oldest = iframeHeightCache.keys().next().value;
    if (oldest !== undefined) iframeHeightCache.delete(oldest);
  }
}

function hashArtifactCode(code: string): string {
  let h = 0;
  for (let i = 0; i < code.length; i++) {
    h = (h * 31 + code.charCodeAt(i)) | 0;
  }
  return `${code.length}_${h}`;
}

const _ArtifactBlock: React.FC<ArtifactBlockProps> = ({
  code,
  isStreaming = false,
  onCopyClick,
}) => {
  const codeKey = useMemo(() => hashArtifactCode(code), [code]);

  const [showSource, setShowSource] = useState(false);
  const [isOpen, setIsOpen] = useState(true);
  const [height, setHeight] = useState(
    () => iframeHeightCache.get(codeKey) ?? DEFAULT_IFRAME_HEIGHT,
  );
  const [error, setError] = useState<string | null>(null);
  const [reloadNonce, setReloadNonce] = useState(0);

  const iframeRef = useRef<HTMLIFrameElement>(null);
  const prevStreaming = useRef(isStreaming);
  const codeKeyRef = useRef(codeKey);
  codeKeyRef.current = codeKey;
  const lastAppliedHeightRef = useRef(height);
  const pendingHeightRef = useRef<number | null>(null);
  const rafRef = useRef<number | null>(null);
  const errorReportsRef = useRef(0);

  const { isDarkMode } = useAppearance();
  const { host } = useConfig();
  const { newFile } = useEventsBusForIDE();
  const isIdeHost = host === "vscode" || host === "jetbrains" || host === "ide";

  useEffect(() => {
    if (prevStreaming.current && !isStreaming) {
      setShowSource(false);
    }
    prevStreaming.current = isStreaming;
  }, [isStreaming]);

  const wrappedHtml = useMemo(() => wrapArtifactHtml(code), [code]);

  useEffect(() => {
    const handler = (event: MessageEvent) => {
      if (event.source !== iframeRef.current?.contentWindow) return;
      const data = event.data as Record<string, unknown> | null;
      if (!data || typeof data.type !== "string") return;

      if (data.type === "refact-artifact-resize") {
        const h = Number(data.height);
        if (!(h > 0)) return;
        // Coalesce resize bursts to one application per frame, always keeping
        // the latest value, and apply hysteresis on the *clamped* height so
        // content taller than the cap cannot oscillate the iframe.
        pendingHeightRef.current = h;
        if (rafRef.current === null) {
          rafRef.current = requestAnimationFrame(() => {
            rafRef.current = null;
            const pending = pendingHeightRef.current;
            pendingHeightRef.current = null;
            if (pending === null) return;
            const next = Math.min(Math.round(pending), MAX_IFRAME_HEIGHT);
            if (
              Math.abs(next - lastAppliedHeightRef.current) <= MIN_HEIGHT_DELTA
            ) {
              return;
            }
            lastAppliedHeightRef.current = next;
            rememberHeight(codeKeyRef.current, next);
            setHeight(next);
          });
        }
        return;
      }

      if (data.type === "refact-artifact-error") {
        const msg = String(data.message).slice(0, MAX_ERROR_MESSAGE_LENGTH);
        setError(msg);
        if (errorReportsRef.current < MAX_ERROR_REPORTS_PER_MOUNT) {
          errorReportsRef.current += 1;
          void reportBuddyFrontendError({
            source: "artifact_iframe",
            error: msg,
            sourceFile: "frontend/artifact_iframe",
            toolName: "artifact_iframe",
          });
        }
      }
    };
    window.addEventListener("message", handler);
    return () => {
      window.removeEventListener("message", handler);
      if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
    };
  }, []);

  useEffect(() => {
    setError(null);
    errorReportsRef.current = 0;
    // Cancel any resize application scheduled for the previous artifact so a
    // late message cannot leak its height into the new one.
    if (rafRef.current !== null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
    pendingHeightRef.current = null;
    const next = iframeHeightCache.get(codeKey) ?? DEFAULT_IFRAME_HEIGHT;
    lastAppliedHeightRef.current = next;
    setHeight(next);
  }, [codeKey]);

  const themeValue = isDarkMode ? "dark" : "light";
  const postThemeToIframe = useCallback(() => {
    iframeRef.current?.contentWindow?.postMessage(
      { type: "refact-artifact-theme", theme: themeValue },
      "*",
    );
  }, [themeValue]);

  useEffect(() => {
    postThemeToIframe();
  }, [postThemeToIframe]);

  const handleToggle = useCallback(() => setIsOpen((v) => !v), []);
  const handleToggleSource = useCallback(() => setShowSource((v) => !v), []);
  const handleReload = useCallback(() => {
    setError(null);
    errorReportsRef.current = 0;
    setReloadNonce((n) => n + 1);
  }, []);

  const handleCopy = useCallback(() => {
    onCopyClick?.(code);
  }, [onCopyClick, code]);

  const handleOpenAsIdeFile = useCallback(() => {
    newFile(code);
  }, [newFile, code]);

  const handleDownload = useCallback(() => {
    const blob = new Blob([wrappedHtml], { type: "text/html" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "artifact.html";
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), BLOB_URL_REVOKE_DELAY_MS);
  }, [wrappedHtml]);

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
    const win = window.open(url, "_blank", "noopener,noreferrer");
    if (!win) {
      // Pop-up blocked: fall back to downloading the preview instead of
      // silently doing nothing.
      handleDownload();
    }
    setTimeout(() => URL.revokeObjectURL(url), BLOB_URL_REVOKE_DELAY_MS);
  }, [wrappedHtml, handleDownload]);

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
      icon={<Icon icon={Globe} size="sm" />}
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
                aria-label={
                  effectiveShowSource ? "Show preview" : "Show source"
                }
                icon={effectiveShowSource ? Eye : Code}
              />
            </Tooltip.Trigger>
            <Tooltip.Content>
              {effectiveShowSource ? "Show preview" : "Show source"}
            </Tooltip.Content>
          </Tooltip>
          <Tooltip>
            <Tooltip.Trigger asChild>
              <IconButton
                size="sm"
                variant="ghost"
                onClick={handleReload}
                disabled={!showIframe}
                aria-label="Re-run preview"
                icon={RefreshCw}
              />
            </Tooltip.Trigger>
            <Tooltip.Content>Re-run preview</Tooltip.Content>
          </Tooltip>
          <div className={styles.tab_bar_spacer} />
          {isIdeHost ? (
            <Tooltip>
              <Tooltip.Trigger asChild>
                <IconButton
                  size="sm"
                  variant="ghost"
                  onClick={handleOpenAsIdeFile}
                  disabled={isStreaming}
                  aria-label="Open as file in IDE"
                  icon={FileCode}
                />
              </Tooltip.Trigger>
              <Tooltip.Content>Open as file in IDE</Tooltip.Content>
            </Tooltip>
          ) : (
            <>
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
            </>
          )}
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
          <div className={classNames("scrollX", styles.source_view)}>
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
            key={`${codeKey}_${reloadNonce}`}
            ref={iframeRef}
            className={styles.iframe}
            srcDoc={wrappedHtml}
            sandbox="allow-scripts"
            referrerPolicy="no-referrer"
            title="HTML Preview"
            style={{ height: `${height}px` }}
            onLoad={postThemeToIframe}
          />
        )}

        {error && <div className={styles.error_bar}>JS Error: {error}</div>}
      </div>
    </ToolCard>
  );
};

export const ArtifactBlock = React.memo(_ArtifactBlock);
