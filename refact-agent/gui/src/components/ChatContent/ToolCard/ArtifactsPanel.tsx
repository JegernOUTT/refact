import { Box, Flex, Text } from "@radix-ui/themes";
import { Download, FileJson, FileText, Images } from "lucide-react";

import type {
  BrowserDownloadInfo,
  BrowserImageArtifact,
  BrowserHarArtifact,
  BrowserPdfArtifact,
} from "../../../services/refact/browser";
import { DialogImage } from "../../DialogImage";
import { Badge, Icon } from "../../ui";
import { AnimatedCollapsible } from "../shared/AnimatedCollapsible";
import styles from "./ArtifactsPanel.module.css";

type DisplayDownload = Omit<BrowserDownloadInfo, "state"> & {
  state: BrowserDownloadInfo["state"] | "failed";
};
type DisplayPdf = BrowserPdfArtifact & { pageCount?: number };

interface ArtifactsPanelProps {
  artifacts?: unknown;
  downloads?: unknown;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNonNegativeNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function artifactValue(value: unknown): Record<string, unknown> | null {
  if (!isRecord(value)) return null;
  return isRecord(value.artifact) ? value.artifact : value;
}

function parseImageArtifact(value: unknown): BrowserImageArtifact | null {
  const artifact = artifactValue(value);
  if (
    !artifact ||
    artifact.kind !== "image" ||
    typeof artifact.mime !== "string" ||
    !artifact.mime.startsWith("image/") ||
    typeof artifact.data !== "string" ||
    artifact.data.length === 0 ||
    artifact.data === "<omitted>" ||
    !isNonNegativeNumber(artifact.width) ||
    !isNonNegativeNumber(artifact.height) ||
    !isNonNegativeNumber(artifact.bytes)
  ) {
    return null;
  }

  return {
    kind: "image",
    mime: artifact.mime,
    data: artifact.data,
    width: artifact.width,
    height: artifact.height,
    bytes: artifact.bytes,
  };
}

function parsePdfArtifact(value: unknown): DisplayPdf | null {
  const artifact = artifactValue(value);
  if (
    !artifact ||
    artifact.kind !== "pdf" ||
    artifact.mime !== "application/pdf" ||
    typeof artifact.path !== "string" ||
    artifact.path.length === 0 ||
    !isNonNegativeNumber(artifact.bytes)
  ) {
    return null;
  }

  const data =
    typeof artifact.data === "string" && artifact.data !== "<omitted>"
      ? artifact.data
      : artifact.data === null
        ? null
        : undefined;
  const pageCount = isNonNegativeNumber(artifact.page_count)
    ? artifact.page_count
    : undefined;

  return {
    kind: "pdf",
    mime: "application/pdf",
    path: artifact.path,
    bytes: artifact.bytes,
    data,
    pageCount,
  };
}

function parseHarArtifact(value: unknown): BrowserHarArtifact | null {
  const artifact = artifactValue(value);
  if (
    !artifact ||
    artifact.kind !== "har" ||
    artifact.mime !== "application/json" ||
    typeof artifact.path !== "string" ||
    artifact.path.length === 0 ||
    !isNonNegativeNumber(artifact.bytes) ||
    !isNonNegativeNumber(artifact.entry_count)
  ) {
    return null;
  }
  return {
    kind: "har",
    mime: "application/json",
    path: artifact.path,
    bytes: artifact.bytes,
    entry_count: artifact.entry_count,
  };
}

function isDownloadState(value: unknown): value is DisplayDownload["state"] {
  return (
    value === "in_progress" ||
    value === "completed" ||
    value === "canceled" ||
    value === "failed"
  );
}

function parseDownload(value: unknown): DisplayDownload | null {
  if (
    !isRecord(value) ||
    typeof value.guid !== "string" ||
    typeof value.url !== "string" ||
    typeof value.frame_id !== "string" ||
    typeof value.suggested_filename !== "string" ||
    typeof value.local_path !== "string" ||
    !isNonNegativeNumber(value.received_bytes) ||
    !isNonNegativeNumber(value.total_bytes) ||
    !isDownloadState(value.state)
  ) {
    return null;
  }

  return {
    guid: value.guid,
    url: value.url,
    frame_id: value.frame_id,
    suggested_filename: value.suggested_filename,
    local_path: value.local_path,
    received_bytes: value.received_bytes,
    total_bytes: value.total_bytes,
    state: value.state,
  };
}

function formatSize(value: number): string {
  if (value < 1_024) return `${Math.round(value)} B`;
  if (value < 1_048_576) return `${(value / 1_024).toFixed(1)} KB`;
  return `${(value / 1_048_576).toFixed(1)} MB`;
}

function fileName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}

function fileUrl(path: string): string {
  if (path.startsWith("file://")) return path;
  const normalized = path.split("\\").join("/");
  return normalized.startsWith("/")
    ? `file://${normalized}`
    : `file:///${normalized}`;
}

function countLabel(count: number, singular: string): string {
  return `${count} ${singular}${count === 1 ? "" : "s"}`;
}

function stateLabel(state: DisplayDownload["state"]): string {
  if (state === "in_progress") return "In progress";
  return state.charAt(0).toUpperCase() + state.slice(1);
}

function stateTone(
  state: DisplayDownload["state"],
): "success" | "warning" | "danger" {
  if (state === "completed") return "success";
  if (state === "in_progress") return "warning";
  return "danger";
}

export function ArtifactsPanel({ artifacts, downloads }: ArtifactsPanelProps) {
  const artifactValues = Array.isArray(artifacts) ? artifacts : [];
  const screenshots = artifactValues
    .map(parseImageArtifact)
    .filter((value): value is BrowserImageArtifact => value !== null);
  const pdfs = artifactValues
    .map(parsePdfArtifact)
    .filter((value): value is DisplayPdf => value !== null);
  const hars = artifactValues
    .map(parseHarArtifact)
    .filter((value): value is BrowserHarArtifact => value !== null);
  const downloadValues = Array.isArray(downloads) ? downloads : [];
  const parsedDownloads = downloadValues
    .map(parseDownload)
    .filter((value): value is DisplayDownload => value !== null);
  const count =
    screenshots.length + pdfs.length + hars.length + parsedDownloads.length;

  if (count === 0) return null;

  const counts = [
    screenshots.length > 0
      ? countLabel(screenshots.length, "screenshot")
      : null,
    pdfs.length > 0 ? countLabel(pdfs.length, "PDF") : null,
    hars.length > 0 ? countLabel(hars.length, "HAR") : null,
    parsedDownloads.length > 0
      ? countLabel(parsedDownloads.length, "download")
      : null,
  ].filter((value): value is string => value !== null);
  const hasFailedDownload = parsedDownloads.some(
    (download) => download.state === "canceled" || download.state === "failed",
  );

  return (
    <Box className={styles.section}>
      <AnimatedCollapsible
        className={styles.panel}
        data-testid="artifacts-panel"
        header={`Artifacts — ${counts.join(", ")}`}
        icon={<Icon icon={Images} />}
        status={hasFailedDownload ? "error" : "success"}
        variant="compact"
      >
        {screenshots.length > 0 ? (
          <Box className={styles.group}>
            <Text className={styles.groupTitle} size="1" weight="bold">
              Screenshots ({screenshots.length})
            </Text>
            <Box className={styles.screenshotGrid}>
              {screenshots.map((screenshot, index) => (
                <Box className={styles.screenshot} key={`screenshot-${index}`}>
                  <DialogImage
                    alt={`Screenshot ${index + 1}`}
                    fallback=""
                    size="9"
                    src={`data:${screenshot.mime};base64,${screenshot.data}`}
                  />
                  <Text className={styles.meta} size="1">
                    {screenshot.width}×{screenshot.height} ·{" "}
                    {formatSize(screenshot.bytes)}
                  </Text>
                </Box>
              ))}
            </Box>
          </Box>
        ) : null}

        {pdfs.length > 0 ? (
          <Box className={styles.group}>
            <Text className={styles.groupTitle} size="1" weight="bold">
              PDFs ({pdfs.length})
            </Text>
            <Box className={styles.rows}>
              {pdfs.map((pdf, index) => {
                const name = fileName(pdf.path);
                const openHref = pdf.data
                  ? `data:${pdf.mime};base64,${pdf.data}`
                  : fileUrl(pdf.path);
                return (
                  <Box className={styles.row} key={`${pdf.path}-${index}`}>
                    <Icon icon={FileText} />
                    <Box className={styles.rowContent}>
                      <a
                        aria-label={`Open PDF ${name}`}
                        className={styles.primaryLink}
                        href={openHref}
                        rel="noreferrer"
                        target="_blank"
                      >
                        {name}
                      </a>
                      <Flex className={styles.meta} gap="2" wrap="wrap">
                        <span>{formatSize(pdf.bytes)}</span>
                        {pdf.pageCount ? (
                          <span>
                            {pdf.pageCount} page{pdf.pageCount === 1 ? "" : "s"}
                          </span>
                        ) : null}
                      </Flex>
                      <a
                        aria-label={`Open local path ${pdf.path}`}
                        className={styles.path}
                        href={fileUrl(pdf.path)}
                        rel="noreferrer"
                        target="_blank"
                      >
                        {pdf.path}
                      </a>
                    </Box>
                  </Box>
                );
              })}
            </Box>
          </Box>
        ) : null}

        {hars.length > 0 ? (
          <Box className={styles.group}>
            <Text className={styles.groupTitle} size="1" weight="bold">
              HARs ({hars.length})
            </Text>
            <Box className={styles.rows}>
              {hars.map((har, index) => (
                <Box className={styles.row} key={`${har.path}-${index}`}>
                  <Icon icon={FileJson} />
                  <Box className={styles.rowContent}>
                    <a
                      aria-label={`Open HAR ${fileName(har.path)}`}
                      className={styles.primaryLink}
                      href={fileUrl(har.path)}
                      rel="noreferrer"
                      target="_blank"
                    >
                      {fileName(har.path)}
                    </a>
                    <Text className={styles.meta} size="1">
                      {har.entry_count} entries · {formatSize(har.bytes)}
                    </Text>
                    <a
                      aria-label={`Open local path ${har.path}`}
                      className={styles.path}
                      href={fileUrl(har.path)}
                      rel="noreferrer"
                      target="_blank"
                    >
                      {har.path}
                    </a>
                  </Box>
                </Box>
              ))}
            </Box>
          </Box>
        ) : null}

        {parsedDownloads.length > 0 ? (
          <Box className={styles.group}>
            <Text className={styles.groupTitle} size="1" weight="bold">
              Downloads ({parsedDownloads.length})
            </Text>
            <Box className={styles.rows}>
              {parsedDownloads.map((download, index) => {
                const failed =
                  download.state === "canceled" || download.state === "failed";
                const size = download.received_bytes || download.total_bytes;
                return (
                  <Box
                    className={styles.row}
                    data-status={failed ? "error" : download.state}
                    data-testid={`download-${index}`}
                    key={`${download.guid}-${index}`}
                  >
                    <Icon icon={Download} />
                    <Box className={styles.rowContent}>
                      <Flex
                        align="center"
                        gap="2"
                        justify="between"
                        wrap="wrap"
                      >
                        <a
                          aria-label={`Open download ${download.suggested_filename}`}
                          className={styles.primaryLink}
                          href={fileUrl(download.local_path)}
                          rel="noreferrer"
                          target="_blank"
                        >
                          {download.suggested_filename}
                        </a>
                        <Badge size="xs" tone={stateTone(download.state)}>
                          {stateLabel(download.state)}
                        </Badge>
                      </Flex>
                      <Text className={styles.meta} size="1">
                        {formatSize(size)}
                      </Text>
                      <span className={styles.url} title={download.url}>
                        {download.url}
                      </span>
                      <a
                        aria-label={`Open local path ${download.local_path}`}
                        className={styles.path}
                        href={fileUrl(download.local_path)}
                        rel="noreferrer"
                        target="_blank"
                      >
                        {download.local_path}
                      </a>
                    </Box>
                  </Box>
                );
              })}
            </Box>
          </Box>
        ) : null}
      </AnimatedCollapsible>
    </Box>
  );
}
