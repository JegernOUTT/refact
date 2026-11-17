import React, { useState } from "react";
import { TriangleAlert } from "lucide-react";

import { Icon, Tooltip } from "../../ui";
import { useOpenFileInApp } from "../../../hooks/useOpenFileInApp";
import { fileTypeIcon } from "../../../features/Workspace/FilesPanel/fileTypeIcon";
import {
  artifactFileName,
  type ArtifactLink,
} from "../../../utils/artifactLinks";
import { useResolvedArtifact, useSeenOnce } from "./useArtifactPreview";
import styles from "./ArtifactLink.module.css";

export type ArtifactLinkChipProps = {
  link: ArtifactLink;
  children?: React.ReactNode;
};

export const ArtifactLinkChip: React.FC<ArtifactLinkChipProps> = ({
  link,
  children,
}) => {
  const [element, setElement] = useState<HTMLButtonElement | null>(null);
  const visible = useSeenOnce(element);
  const { currentData, isError } = useResolvedArtifact(link, visible);
  const { canOpen, openFile } = useOpenFileInApp();
  const fileName = artifactFileName(link.relPath);
  const problem = currentData?.exists === false || (!currentData && isError);
  const openable = canOpen && currentData?.exists === true;
  const tooltip =
    currentData?.exists === false
      ? `File not found: ${link.relPath}`
      : !currentData && isError
        ? `Artifact unavailable: ${link.relPath}`
        : link.relPath;

  return (
    <Tooltip content={tooltip}>
      <button
        className={styles.chip}
        data-state={problem ? "missing" : "ready"}
        data-testid="artifact-chip"
        aria-disabled={!openable}
        ref={setElement}
        onClick={() => {
          if (openable) openFile({ path: currentData.path });
        }}
        type="button"
      >
        <Icon
          icon={problem ? TriangleAlert : fileTypeIcon(fileName)}
          size="sm"
          tone={problem ? "warning" : "muted"}
        />
        <span className={styles.chipLabel}>{children ?? fileName}</span>
      </button>
    </Tooltip>
  );
};
