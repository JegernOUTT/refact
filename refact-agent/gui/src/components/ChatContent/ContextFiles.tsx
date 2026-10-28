import React, { useCallback } from "react";
import ReactMarkDown from "react-markdown";
import { Archive, FileText, Zap, type LucideIcon } from "lucide-react";
import { Icon } from "../ui";
import { ChatContextFile } from "../../services/refact";
import { ShikiCodeBlock } from "../Markdown/ShikiCodeBlock";
import { filename } from "../../utils";
import { useOpenFileInApp } from "../../hooks/useOpenFileInApp";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { push } from "../../features/Pages/pagesSlice";
import { TranscriptItem } from "./TranscriptItem";
import { formatTextSize } from "./ContextFiles.utils";
import styles from "./ContextFiles.module.css";

export const Markdown: React.FC<{
  children: string;
}> = (props) => {
  return (
    <ReactMarkDown
      components={{
        code({ style: _style, color: _color, ...codeProps }) {
          return <ShikiCodeBlock {...codeProps} showLineNumbers={false} />;
        },
      }}
      {...props}
    />
  );
};

const PREVIEW_LINES = 6;
function getExtensionFromName(name: string): string {
  const dot = name.lastIndexOf(".");
  if (dot === -1) return "";
  return name.substring(dot + 1).replace(/:\d*-\d*/, "");
}

type ContextVariant =
  | "default"
  | "enrichment"
  | "project_context"
  | "memories_context";

function contextVariant(toolCallId: string | undefined): ContextVariant {
  if (toolCallId === "knowledge_enrichment") return "enrichment";
  if (toolCallId === "project_context") return "project_context";
  if (toolCallId === "memories_context") return "memories_context";
  return "default";
}

function formatFileName(
  filePath: string,
  line1?: number,
  line2?: number,
): string {
  const name = filename(filePath);
  if (line1 && line2) return `${name}:${line1}-${line2}`;
  return name;
}

function extractEnrichmentDisplayName(filePath: string): string {
  const fileName = filename(filePath);
  const memoryMatch = fileName.match(
    /^\d{4}-\d{2}-\d{2}_\d{6}_[a-f0-9]+_(.+)\.md$/,
  );
  if (memoryMatch) return memoryMatch[1].replace(/-/g, " ");
  if (/^[a-f0-9-]{36}\.json$/.test(fileName)) return "Past conversation";
  return fileName;
}

const PROJECT_MARKERS = [
  ".vscode",
  ".idea",
  ".cursor",
  ".windsurf",
  ".github",
  ".refact",
  ".zed",
  ".fleet",
  ".claude",
];

function extractProjectContextDisplayName(filePath: string): string {
  const parts = filePath.split("/");
  const markerIndex = parts.findIndex((part) => PROJECT_MARKERS.includes(part));
  if (markerIndex !== -1) return parts.slice(markerIndex).join("/");
  return parts.length >= 2 ? parts.slice(-2).join("/") : filename(filePath);
}

function displayName(file: ChatContextFile, variant: ContextVariant): string {
  if (variant === "enrichment") {
    return extractEnrichmentDisplayName(file.file_name);
  }
  if (variant === "project_context") {
    return extractProjectContextDisplayName(file.file_name);
  }
  return formatFileName(file.file_name, file.line1, file.line2);
}

function previewOf(content: string): string {
  return content.split("\n").slice(0, PREVIEW_LINES).join("\n");
}

const VARIANT_VERBS: Record<ContextVariant, string> = {
  default: "Context",
  enrichment: "Memories",
  project_context: "Context",
  memories_context: "Memories",
};

const VARIANT_ICONS: Record<ContextVariant, LucideIcon> = {
  default: FileText,
  enrichment: Zap,
  project_context: Archive,
  memories_context: Zap,
};

type OpenFile = (file: { file_path: string; line?: number }) => void;

const ContextFileEntry: React.FC<{
  file: ChatContextFile;
  variant: ContextVariant;
  canOpen: boolean;
  onOpenFile: OpenFile;
}> = ({ file, variant, canOpen, onOpenFile }) => {
  const isClickable =
    canOpen ||
    file.file_name.startsWith("skill://") ||
    file.file_name.startsWith("skills://");
  const name = displayName(file, variant);
  const relevance = file.usefulness ? Math.round(file.usefulness) : null;
  const handleClick = useCallback(() => {
    onOpenFile({ file_path: file.file_name, line: file.line1 });
  }, [onOpenFile, file.file_name, file.line1]);

  return (
    <li className={styles.file} data-testid="context-file">
      <div className={styles.fileHeader}>
        {isClickable ? (
          <button
            type="button"
            className={styles.fileLink}
            title={file.file_name}
            onClick={handleClick}
          >
            {name}
          </button>
        ) : (
          <span className={styles.fileName} title={file.file_name}>
            {name}
          </span>
        )}
        {relevance !== null && (
          <span className={styles.relevance}>{relevance}%</span>
        )}
      </div>
      {file.file_content.trim().length > 0 && (
        <div className={styles.preview}>
          <ShikiCodeBlock showLineNumbers={false}>
            {`\`\`\`${getExtensionFromName(file.file_name)}\n${previewOf(
              file.file_content,
            )}\n\`\`\``}
          </ShikiCodeBlock>
        </div>
      )}
    </li>
  );
};

const _ContextFiles: React.FC<{
  files: ChatContextFile[];
  toolCallId?: string;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}> = ({ files, toolCallId, open, onOpenChange }) => {
  const { canOpen, openFile } = useOpenFileInApp();
  const dispatch = useAppDispatch();

  const handleOpenFile = useCallback<OpenFile>(
    (file) => {
      if (file.file_path.startsWith("skill://")) {
        const skillName = file.file_path.slice("skill://".length);
        dispatch(
          push({ name: "extensions", tab: "skills", itemId: skillName }),
        );
        return;
      }
      if (file.file_path.startsWith("skills://")) {
        dispatch(push({ name: "extensions", tab: "skills" }));
        return;
      }
      openFile({ path: file.file_path, line: file.line });
    },
    [dispatch, openFile],
  );

  if (!Array.isArray(files) || files.length === 0) return null;
  const variant = contextVariant(toolCallId);
  const totalSize = formatTextSize(
    files.map((file) => file.file_content).join(""),
  );
  const target =
    variant === "default" && files.length === 1
      ? displayName(files[0], variant)
      : `${files.length} ${files.length === 1 ? "file" : "files"}`;

  return (
    <TranscriptItem
      data-testid="context-files"
      data-variant={variant}
      icon={<Icon icon={VARIANT_ICONS[variant]} size="sm" />}
      verb={VARIANT_VERBS[variant]}
      target={target}
      meta={totalSize}
      open={open}
      onOpenChange={onOpenChange}
      sectionLabels={{ output: "" }}
      output={
        <ul className={styles.list}>
          {files.map((file, index) => (
            <ContextFileEntry
              key={`${file.file_name}-${index}`}
              file={file}
              variant={variant}
              canOpen={canOpen}
              onOpenFile={handleOpenFile}
            />
          ))}
        </ul>
      }
    />
  );
};

export const ContextFiles = React.memo(_ContextFiles);
