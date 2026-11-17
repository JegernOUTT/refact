import React, { useState } from "react";
import { Copy, RotateCw, TriangleAlert } from "lucide-react";

import { Icon, IconButton, Skeleton } from "../../ui";
import { useCopyToClipboard } from "../../../hooks/useCopyToClipboard";
import { useOpenFileInApp } from "../../../hooks/useOpenFileInApp";
import { fileTypeIcon } from "../../../features/Workspace/FilesPanel/fileTypeIcon";
import { formatBytes } from "../../../features/Performance/performanceFormatters";
import {
  artifactFileName,
  type ArtifactLink,
} from "../../../utils/artifactLinks";
import { artifactKind, artifactTypeLabel } from "./artifactKind";
import {
  ArtifactPreview,
  type RenderArtifactMarkdown,
} from "./ArtifactPreview";
import {
  useArtifactPreview,
  useResolvedArtifact,
  useSeenOnce,
} from "./useArtifactPreview";
import styles from "./ArtifactLink.module.css";

export type ArtifactLinkCardProps = {
  link: ArtifactLink;
  title: string;
  renderMarkdown: RenderArtifactMarkdown;
};

export const ArtifactLinkCard: React.FC<ArtifactLinkCardProps> = ({
  link,
  title,
  renderMarkdown,
}) => {
  const [element, setElement] = useState<HTMLDivElement | null>(null);
  const visible = useSeenOnce(element);
  const { currentData, isError, isLoading, isFetching, refetch } =
    useResolvedArtifact(link, visible);
  const { canOpen, openFile } = useOpenFileInApp();
  const copy = useCopyToClipboard();
  const fileName = artifactFileName(link.relPath);
  const label = title || fileName;
  const kind = artifactKind(fileName);
  const { preview, status } = useArtifactPreview(currentData, kind, visible);
  const FileIcon = fileTypeIcon(fileName);

  const missing = currentData?.exists === false;
  const unavailable = !currentData && isError;
  const loading =
    !currentData && !isError && (!visible || isLoading || isFetching);
  const openable = canOpen && currentData?.exists === true;

  const meta = [
    artifactTypeLabel(fileName),
    currentData?.exists && currentData.bytes !== null
      ? formatBytes(currentData.bytes)
      : null,
    link.relPath,
  ]
    .filter(Boolean)
    .join(" · ");

  let previewArea: React.ReactNode = null;
  if (preview) {
    previewArea = (
      <ArtifactPreview
        preview={preview}
        renderMarkdown={renderMarkdown}
        title={label}
      />
    );
  } else if (loading || status === "loading" || status === "idle") {
    previewArea =
      kind === "none" ? null : (
        <div className={styles.previewSkeleton}>
          <Skeleton height="100%" radius="none" />
        </div>
      );
  } else if (currentData?.exists && kind === "none") {
    previewArea = <div className={styles.noPreview}>No preview</div>;
  }

  let statusLine: React.ReactNode = <span className={styles.meta}>{meta}</span>;
  if (missing) {
    statusLine = (
      <span className={styles.meta}>File not found · {link.relPath}</span>
    );
  } else if (unavailable) {
    statusLine = (
      <span className={styles.meta}>Artifact unavailable · {link.relPath}</span>
    );
  }

  return (
    <div
      className={styles.card}
      data-state={
        missing
          ? "missing"
          : unavailable
            ? "error"
            : loading
              ? "loading"
              : "ready"
      }
      data-testid="artifact-card"
      ref={setElement}
    >
      {previewArea}
      <div className={styles.footer}>
        <span className={styles.typeTile}>
          <Icon
            icon={missing || unavailable ? TriangleAlert : FileIcon}
            tone={missing || unavailable ? "warning" : "muted"}
          />
        </span>
        <span className={styles.footerText}>
          <button
            aria-label={`Open ${label}`}
            className={styles.primary}
            disabled={!openable}
            onClick={() => {
              if (currentData) openFile({ path: currentData.path });
            }}
            title={link.relPath}
            type="button"
          >
            <span className={styles.title}>{label}</span>
          </button>
          {statusLine}
        </span>
        <span className={styles.actions}>
          {unavailable && (
            <IconButton
              aria-label="Retry"
              icon={RotateCw}
              loading={isFetching}
              onClick={() => void refetch()}
              size="sm"
            />
          )}
          {currentData?.exists && (
            <IconButton
              aria-label="Copy path"
              icon={Copy}
              onClick={() => copy(currentData.path)}
              size="sm"
            />
          )}
        </span>
      </div>
    </div>
  );
};
