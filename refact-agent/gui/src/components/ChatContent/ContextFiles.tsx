import React, { useState, useCallback } from "react";
import { useStoredOpen } from "./useStoredOpen";
import { Flex, Box, Text } from "@radix-ui/themes";
import classNames from "classnames";
import ReactMarkDown from "react-markdown";
import { Archive, BookOpen, FileText, Zap } from "lucide-react";
import { Icon } from "../ui";
import { ChatContextFile } from "../../services/refact";
import { ShikiCodeBlock } from "../Markdown/ShikiCodeBlock";
import { filename } from "../../utils";
import { useEventsBusForIDE, useAppDispatch } from "../../hooks";
import { push } from "../../features/Pages/pagesSlice";
import { useDelayedUnmount } from "../shared/useDelayedUnmount";
import styles from "./ContextFiles.module.css";

// Re-export Markdown for backward compatibility
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

function isInstructionFile(filePath: string): boolean {
  const lower = filePath.toLowerCase();
  return (
    lower.includes("agents.md") ||
    lower.includes("claude.md") ||
    lower.includes("gemini.md") ||
    lower.includes("refact.md") ||
    lower.includes(".cursorrules") ||
    lower.includes(".cursor/rules") ||
    lower.includes("global_rules.md") ||
    lower.includes(".windsurf/rules") ||
    lower.includes("copilot-instructions") ||
    lower.includes(".github/instructions") ||
    lower.includes(".aider.conf") ||
    lower.includes(".refact/instructions")
  );
}

function isIdeSettingFile(filePath: string): boolean {
  const lower = filePath.toLowerCase();
  return (
    lower.includes(".vscode/") ||
    lower.includes(".idea/") ||
    lower.includes(".zed/") ||
    lower.includes(".fleet/") ||
    lower.includes(".claude/")
  );
}

function formatFileName(
  filePath: string,
  line1?: number,
  line2?: number,
): string {
  const name = filename(filePath);
  if (line1 && line2 && line1 !== 0 && line2 !== 0) {
    return `${name}:${line1}-${line2}`;
  }
  return name;
}

function extractEnrichmentDisplayName(filePath: string): string {
  const fileName = filename(filePath);

  const memoryMatch = fileName.match(
    /^\d{4}-\d{2}-\d{2}_\d{6}_[a-f0-9]+_(.+)\.md$/,
  );
  if (memoryMatch) {
    return memoryMatch[1].replace(/-/g, " ");
  }

  const trajectoryMatch = fileName.match(/^[a-f0-9-]{36}\.json$/);
  if (trajectoryMatch) {
    return "Past conversation";
  }

  return fileName;
}

function extractProjectContextDisplayName(filePath: string): string {
  const parts = filePath.split("/");

  const markers = [
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
  for (let i = 0; i < parts.length; i++) {
    if (markers.includes(parts[i])) {
      return parts.slice(i).join("/");
    }
  }

  const fileName = filename(filePath);
  const instructionFiles = [
    "AGENTS.md",
    "CLAUDE.md",
    "GEMINI.md",
    "REFACT.md",
    ".cursorrules",
    "global_rules.md",
    "copilot-instructions.md",
    ".aider.conf.yml",
  ];
  if (
    instructionFiles.some((f) => fileName.toLowerCase() === f.toLowerCase())
  ) {
    return fileName;
  }

  if (parts.length >= 2) {
    return parts.slice(-2).join("/");
  }

  return fileName;
}

const FileItem: React.FC<{
  file: ChatContextFile;
  onOpenFile: (file: { file_path: string; line?: number }) => Promise<void>;
  variant: ContextVariant;
}> = ({ file, onOpenFile, variant }) => {
  const storeKey = `ctxfile:${file.file_name}:${file.line1 || 0}-${
    file.line2 || 0
  }`;
  const [isOpen, toggleOpen] = useStoredOpen(storeKey, false);
  const { shouldRender, isAnimatingOpen } = useDelayedUnmount(isOpen, 200);
  const extension = getExtensionFromName(file.file_name);

  const displayName =
    variant === "enrichment"
      ? extractEnrichmentDisplayName(file.file_name)
      : variant === "project_context"
        ? extractProjectContextDisplayName(file.file_name)
        : formatFileName(file.file_name, file.line1, file.line2);

  const relevance = file.usefulness ? Math.round(file.usefulness) : null;

  const handleToggle = useCallback(() => {
    toggleOpen();
  }, [toggleOpen]);

  const handleFileClick = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      void onOpenFile({
        file_path: file.file_name,
        line: file.line1,
      });
    },
    [onOpenFile, file.file_name, file.line1],
  );

  return (
    <div className={styles.fileItem}>
      <Flex
        className={styles.fileHeader}
        align="center"
        gap="2"
        onClick={handleToggle}
      >
        <Text size="1" className={styles.fileName} onClick={handleFileClick}>
          {displayName}
        </Text>
        {relevance !== null && (
          <Text size="1" className={styles.relevance}>
            {relevance}%
          </Text>
        )}
      </Flex>

      {shouldRender && (
        <div
          className={classNames(
            "rf-expand-grid",
            isAnimatingOpen && "is-open",
            styles.contentWrapper,
            isAnimatingOpen && styles.contentWrapperOpen,
          )}
        >
          <div className={styles.contentInner}>
            <Box className={styles.fileContent}>
              <ShikiCodeBlock showLineNumbers={false}>
                {`\`\`\`${extension}\n${file.file_content}\n\`\`\``}
              </ShikiCodeBlock>
            </Box>
          </div>
        </div>
      )}
    </div>
  );
};

const FileSection: React.FC<{
  icon: React.ReactNode;
  title: string;
  files: ChatContextFile[];
  onOpenFile: (file: { file_path: string; line?: number }) => Promise<void>;
  variant: ContextVariant;
}> = ({ icon, title, files, onOpenFile, variant }) => {
  return (
    <Box className={styles.section}>
      <Flex align="center" gap="2" className={styles.sectionHeader}>
        <span className={styles.sectionIcon}>{icon}</span>
        <Text size="1" className={styles.sectionTitle}>
          {title}
        </Text>
      </Flex>
      <Flex direction="column" gap="1" className={styles.sectionContent}>
        {files.map((file, index) => (
          <FileItem
            key={file.file_name + index}
            file={file}
            onOpenFile={onOpenFile}
            variant={variant}
          />
        ))}
      </Flex>
    </Box>
  );
};

const FilesContent: React.FC<{
  files: ChatContextFile[];
  onOpenFile: (file: { file_path: string; line?: number }) => Promise<void>;
  variant: ContextVariant;
}> = ({ files, onOpenFile, variant }) => {
  if (files.length === 0) return null;

  if (variant === "enrichment") {
    const memories = files.filter((f) =>
      f.file_name.includes("/.refact/memories/"),
    );
    const trajectories = files.filter((f) =>
      f.file_name.includes("/.refact/trajectories/"),
    );
    const other = files.filter(
      (f) =>
        !f.file_name.includes("/.refact/memories/") &&
        !f.file_name.includes("/.refact/trajectories/"),
    );

    return (
      <Flex direction="column" gap="2">
        {memories.length > 0 && (
          <FileSection
            icon={<Icon icon={BookOpen} size="sm" tone="muted" />}
            title="Knowledge"
            files={memories}
            onOpenFile={onOpenFile}
            variant={variant}
          />
        )}
        {trajectories.length > 0 && (
          <FileSection
            icon={<Icon icon={Archive} size="sm" tone="muted" />}
            title="Past Conversations"
            files={trajectories}
            onOpenFile={onOpenFile}
            variant={variant}
          />
        )}
        {other.length > 0 && (
          <FileSection
            icon={<Icon icon={FileText} size="sm" tone="muted" />}
            title="Related"
            files={other}
            onOpenFile={onOpenFile}
            variant={variant}
          />
        )}
      </Flex>
    );
  }

  if (variant === "project_context") {
    const instructions = files.filter((f) => isInstructionFile(f.file_name));
    const ideSettings = files.filter((f) => isIdeSettingFile(f.file_name));
    const other = files.filter(
      (f) => !isInstructionFile(f.file_name) && !isIdeSettingFile(f.file_name),
    );

    return (
      <Flex direction="column" gap="2">
        {instructions.length > 0 && (
          <FileSection
            icon={<Icon icon={BookOpen} size="sm" tone="muted" />}
            title="Instructions"
            files={instructions}
            onOpenFile={onOpenFile}
            variant={variant}
          />
        )}
        {ideSettings.length > 0 && (
          <FileSection
            icon={<Icon icon={Archive} size="sm" tone="muted" />}
            title="IDE Settings"
            files={ideSettings}
            onOpenFile={onOpenFile}
            variant={variant}
          />
        )}
        {other.length > 0 && (
          <FileSection
            icon={<Icon icon={FileText} size="sm" tone="muted" />}
            title="Other"
            files={other}
            onOpenFile={onOpenFile}
            variant={variant}
          />
        )}
      </Flex>
    );
  }

  return (
    <Flex direction="column" gap="1">
      {files.map((file, index) => (
        <FileItem
          key={file.file_name + index}
          file={file}
          onOpenFile={onOpenFile}
          variant={variant}
        />
      ))}
    </Flex>
  );
};

const _ContextFiles: React.FC<{
  files: ChatContextFile[];
  toolCallId?: string;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}> = ({ files, toolCallId, open: controlledOpen, onOpenChange }) => {
  const [internalOpen, setInternalOpen] = useState(false);
  const { queryPathThenOpenFile } = useEventsBusForIDE();
  const dispatch = useAppDispatch();

  const handleOpenFile = useCallback(
    async (file: { file_path: string; line?: number }) => {
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
      await queryPathThenOpenFile(file);
    },
    [dispatch, queryPathThenOpenFile],
  );

  const isControlled = controlledOpen !== undefined;
  const isOpen = isControlled ? controlledOpen : internalOpen;
  const { shouldRender, isAnimatingOpen } = useDelayedUnmount(isOpen, 200);

  const handleToggle = useCallback(() => {
    if (isControlled && onOpenChange) {
      onOpenChange(!controlledOpen);
    } else {
      setInternalOpen((prev) => !prev);
    }
  }, [isControlled, onOpenChange, controlledOpen]);

  if (!Array.isArray(files) || files.length === 0) return null;

  const variant: ContextVariant =
    toolCallId === "knowledge_enrichment"
      ? "enrichment"
      : toolCallId === "project_context"
        ? "project_context"
        : toolCallId === "memories_context"
          ? "memories_context"
          : "default";


  const icon =
    variant === "enrichment" ? (
      <Icon icon={Zap} size="sm" tone="accent" />
    ) : variant === "project_context" ? (
      <Icon icon={Archive} size="sm" tone="muted" />
    ) : variant === "memories_context" ? (
      <Icon icon={Zap} size="sm" tone="accent" />
    ) : (
      <Icon icon={FileText} size="sm" tone="muted" />
    );
  const label =
    variant === "enrichment"
      ? `Memories (${files.length})`
      : variant === "project_context"
        ? `Project context (${files.length})`
        : variant === "memories_context"
          ? `User memories (${files.length})`
          : files
              .map((f) => formatFileName(f.file_name, f.line1, f.line2))
              .join(", ");

  return (
    <div className={styles.card}>
      <Flex
        className={styles.header}
        align="center"
        gap="2"
        onClick={handleToggle}
      >
        <span className={styles.icon}>{icon}</span>
        <Text size="1" className={styles.summary}>
          {label}
        </Text>
      </Flex>

      {shouldRender && (
        <div
          className={classNames(
            "rf-expand-grid",
            isAnimatingOpen && "is-open",
            styles.contentWrapper,
            isAnimatingOpen && styles.contentWrapperOpen,
          )}
        >
          <div className={styles.contentInner}>
            <Box className={styles.content}>
              <FilesContent
                files={files}
                onOpenFile={handleOpenFile}
                variant={variant}
              />
            </Box>
          </div>
        </div>
      )}
    </div>
  );
};

export const ContextFiles = React.memo(_ContextFiles);
