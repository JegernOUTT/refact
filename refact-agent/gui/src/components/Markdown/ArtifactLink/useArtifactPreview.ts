import { useEffect, useMemo, useState } from "react";
import { skipToken } from "@reduxjs/toolkit/query/react";

import { useAppSelector } from "../../../hooks/useAppSelector";
import { useIsIntersecting } from "../../ScrollArea/useIsIntersecting";
import { isPathWithinWorkspaceRoots } from "../../../features/Workspace/FilesPanel/filesPanelSlice";
import {
  imageDataUrl,
  useReadFileQuery,
  type ReadFileRequest,
  type ReadFileResponse,
} from "../../../services/refact/files";
import {
  useResolveArtifactQuery,
  type ArtifactResolveResponse,
} from "../../../services/refact/artifacts";
import type { ArtifactLink } from "../../../utils/artifactLinks";
import {
  parseDelimitedPreview,
  previewByteLimit,
  previewLineCount,
  svgDataUrl,
  type ArtifactKind,
} from "./artifactKind";
import { buildHtmlPreviewDocument } from "./htmlPreviewDocument";

export type ArtifactPreviewData =
  | { type: "image"; src: string }
  | { type: "svg"; src: string }
  | { type: "html"; srcDoc: string }
  | { type: "markdown"; content: string }
  | { type: "table"; rows: string[][] }
  | { type: "text"; content: string };

export type ArtifactPreviewStatus =
  | "idle"
  | "skipped"
  | "loading"
  | "ready"
  | "failed";

const EMPTY_ROOTS: string[] = [];
const VISIBILITY_OPTIONS: IntersectionObserverInit = { rootMargin: "200px" };

export function useResolvedArtifact(link: ArtifactLink, visible: boolean) {
  return useResolveArtifactQuery(visible ? link : skipToken, {
    refetchOnMountOrArgChange: true,
  });
}

export function useSeenOnce(element: HTMLElement | null): boolean {
  const intersecting = useIsIntersecting(element, VISIBILITY_OPTIONS);
  const [seen, setSeen] = useState(false);
  useEffect(() => {
    if (intersecting) setSeen(true);
  }, [intersecting]);
  return seen;
}

function headLines(content: string, count: number): string {
  return content.split("\n").slice(0, count).join("\n");
}

function previewFromFile(
  kind: ArtifactKind,
  file: ReadFileResponse,
): ArtifactPreviewData | null {
  if (kind === "image") {
    const src = imageDataUrl(file.image_mime, file.image_data);
    return src ? { type: "image", src } : null;
  }
  if (file.binary) return null;
  const lines = previewLineCount(kind);
  switch (kind) {
    case "svg":
      return file.truncated
        ? null
        : { type: "svg", src: svgDataUrl(file.content) };
    case "html":
      return file.truncated
        ? null
        : { type: "html", srcDoc: buildHtmlPreviewDocument(file.content) };
    case "markdown":
      return {
        type: "markdown",
        content: headLines(file.content, lines ?? 0),
      };
    case "csv":
    case "tsv": {
      const rows = parseDelimitedPreview(
        headLines(file.content, lines ?? 0),
        kind === "csv" ? "," : "\t",
      );
      return rows.length > 0 ? { type: "table", rows } : null;
    }
    case "text":
      return { type: "text", content: headLines(file.content, lines ?? 0) };
    default:
      return null;
  }
}

export function useArtifactPreview(
  resolved: ArtifactResolveResponse | undefined,
  kind: ArtifactKind,
  visible: boolean,
): { preview: ArtifactPreviewData | null; status: ArtifactPreviewStatus } {
  const workspaceRoots = useAppSelector(
    (state) => state.current_project.workspaceRoots ?? EMPTY_ROOTS,
  );
  const eligible =
    resolved !== undefined &&
    resolved.exists &&
    kind !== "none" &&
    (resolved.bytes === null || resolved.bytes <= previewByteLimit(kind));

  const request = useMemo((): ReadFileRequest | null => {
    if (!resolved || !eligible) return null;
    const revision = `${String(resolved.modified_at_ms)}:${String(
      resolved.bytes,
    )}`;
    const manual =
      workspaceRoots.length > 0 &&
      !isPathWithinWorkspaceRoots(resolved.path, workspaceRoots);
    if (manual) return { path: resolved.path, manual: true, revision };
    const lines = previewLineCount(kind);
    return lines === null
      ? { path: resolved.path, revision }
      : { path: resolved.path, lineStart: 1, lineEnd: lines, revision };
  }, [eligible, kind, resolved, workspaceRoots]);

  const { currentData: data, isError } = useReadFileQuery(
    request && visible ? request : skipToken,
  );

  const preview = useMemo(
    () => (data ? previewFromFile(kind, data) : null),
    [data, kind],
  );

  if (!request) return { preview: null, status: "skipped" };
  if (!visible) return { preview: null, status: "idle" };
  if (isError) return { preview: null, status: "failed" };
  if (!data) return { preview: null, status: "loading" };
  return preview
    ? { preview, status: "ready" }
    : { preview: null, status: "failed" };
}
