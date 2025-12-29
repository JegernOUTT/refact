import React from "react";
import { Flex, Container, Box, Text } from "@radix-ui/themes";
import { ChatContextFile } from "../../services/refact";
import * as Collapsible from "@radix-ui/react-collapsible";
import { Link } from "../Link";
import ReactMarkDown from "react-markdown";
import { MarkdownCodeBlock } from "../Markdown/CodeBlock";
import { Chevron } from "../Collapsible";
import { filename } from "../../utils";
import { useEventsBusForIDE } from "../../hooks";

export const Markdown: React.FC<{
  children: string;
  startingLineNumber?: number;
}> = ({ startingLineNumber, ...props }) => {
  return (
    <ReactMarkDown
      components={{
        code({ style: _style, color: _color, ...codeProps }) {
          return (
            <MarkdownCodeBlock
              {...codeProps}
              showLineNumbers={false}
              startingLineNumber={startingLineNumber}
            />
          );
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

type ContextVariant = "default" | "enrichment" | "project_context";

const FilesContent: React.FC<{
  files: ChatContextFile[];
  onOpenFile: (file: { file_path: string; line?: number }) => Promise<void>;
  variant: ContextVariant;
}> = ({ files, onOpenFile, variant }) => {
  if (files.length === 0) return null;

  if (variant === "enrichment") {
    const memories = files.filter(f => f.file_name.includes("/.refact/memories/"));
    const trajectories = files.filter(f => f.file_name.includes("/.refact/trajectories/"));
    const other = files.filter(f =>
      !f.file_name.includes("/.refact/memories/") &&
      !f.file_name.includes("/.refact/trajectories/")
    );

    return (
      <Flex direction="column" gap="2">
        {memories.length > 0 && (
          <FileSection icon="📝" title="Knowledge" files={memories} onOpenFile={onOpenFile} variant={variant} />
        )}
        {trajectories.length > 0 && (
          <FileSection icon="💬" title="Past Conversations" files={trajectories} onOpenFile={onOpenFile} variant={variant} />
        )}
        {other.length > 0 && (
          <FileSection icon="📄" title="Related" files={other} onOpenFile={onOpenFile} variant={variant} />
        )}
      </Flex>
    );
  }

  if (variant === "project_context") {
    const instructions = files.filter(f => isInstructionFile(f.file_name));
    const ideSettings = files.filter(f => isIdeSettingFile(f.file_name));
    const other = files.filter(f => !isInstructionFile(f.file_name) && !isIdeSettingFile(f.file_name));

    return (
      <Flex direction="column" gap="2">
        {instructions.length > 0 && (
          <FileSection icon="📝" title="Instructions" files={instructions} onOpenFile={onOpenFile} variant={variant} />
        )}
        {ideSettings.length > 0 && (
          <FileSection icon="⚙️" title="IDE Settings" files={ideSettings} onOpenFile={onOpenFile} variant={variant} />
        )}
        {other.length > 0 && (
          <FileSection icon="📄" title="Other" files={other} onOpenFile={onOpenFile} variant={variant} />
        )}
      </Flex>
    );
  }

  return (
    <Flex direction="column" gap="1">
      {files.map((file, index) => (
        <FileCard
          key={file.file_name + index}
          file={file}
          onOpenFile={onOpenFile}
          variant="default"
        />
      ))}
    </Flex>
  );
};

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
    lower.includes(".refact/project_summary") ||
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

export const ContextFiles: React.FC<{
  files: ChatContextFile[];
  toolCallId?: string;
}> = ({ files, toolCallId }) => {
  const [open, setOpen] = React.useState(false);
  const { queryPathThenOpenFile } = useEventsBusForIDE();

  if (!Array.isArray(files) || files.length === 0) return null;

  const variant: ContextVariant =
    toolCallId === "knowledge_enrichment" ? "enrichment" :
    toolCallId === "project_context" ? "project_context" :
    "default";

  const icon =
    variant === "enrichment" ? "🧠" :
    variant === "project_context" ? "📁" :
    "📎";

  const label =
    variant === "enrichment" ? `${files.length} memories` :
    variant === "project_context" ? `Project context (${files.length})` :
    `${files.length} file${files.length > 1 ? "s" : ""}`;

  return (
    <Container>
      <Collapsible.Root open={open} onOpenChange={setOpen}>
        <Collapsible.Trigger asChild>
          <Flex gap="2" align="start" py="2" style={{ cursor: "pointer" }}>
            <Text weight="light" size="1" style={{ color: "var(--gray-10)" }}>
              {icon} {label}
            </Text>
            <Chevron open={open} />
          </Flex>
        </Collapsible.Trigger>
        <Collapsible.Content>
          <FilesContent
            files={files}
            onOpenFile={queryPathThenOpenFile}
            variant={variant}
          />
        </Collapsible.Content>
      </Collapsible.Root>
    </Container>
  );
};

const FileSection: React.FC<{
  icon: string;
  title: string;
  files: ChatContextFile[];
  onOpenFile: (file: { file_path: string; line?: number }) => Promise<void>;
  variant: ContextVariant;
}> = ({ icon, title, files, onOpenFile, variant }) => {
  return (
    <Box>
      <Text size="1" weight="light" style={{ color: "var(--gray-9)" }}>
        {icon} {title}
      </Text>
      <Flex direction="column" gap="1" mt="1">
        {files.map((file, index) => (
          <FileCard
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

const FileCard: React.FC<{
  file: ChatContextFile;
  onOpenFile: (file: { file_path: string; line?: number }) => Promise<void>;
  variant: ContextVariant;
}> = ({ file, onOpenFile, variant }) => {
  const [showContent, setShowContent] = React.useState(false);
  const extension = getExtensionFromName(file.file_name);
  const start = file.line1 || 1;

  const displayName =
    variant === "enrichment" ? extractEnrichmentDisplayName(file.file_name) :
    variant === "project_context" ? extractProjectContextDisplayName(file.file_name) :
    formatFileName(file.file_name, file.line1, file.line2);
  const relevance = file.usefulness ? Math.round(file.usefulness) : null;

  const preview = file.file_content.slice(0, 100).replace(/\n/g, " ") +
    (file.file_content.length > 100 ? "..." : "");

  return (
    <Box pl="2" style={{ borderLeft: "1px solid var(--gray-a4)" }}>
      <Flex justify="between" align="start" gap="2">
        <Box style={{ flex: 1, minWidth: 0 }}>
          <Flex align="center" gap="2">
            <Link
              onClick={(e) => {
                e.preventDefault();
                void onOpenFile({ file_path: file.file_name, line: file.line1 });
              }}
              style={{ cursor: "pointer" }}
            >
              <Text size="1" weight="light" style={{ color: "var(--gray-11)" }}>
                {displayName}
              </Text>
            </Link>
            {relevance !== null && (
              <Text size="1" style={{ color: "var(--gray-9)" }}>
                {relevance}%
              </Text>
            )}
          </Flex>
          <Text size="1" style={{ color: "var(--gray-9)" }}>
            {preview}
          </Text>
        </Box>
        <Box
          style={{ cursor: "pointer", flexShrink: 0 }}
          onClick={() => setShowContent(!showContent)}
        >
          <Chevron open={showContent} />
        </Box>
      </Flex>
      {showContent && (
        <Box mt="2" style={{ maxHeight: "300px", overflow: "auto" }}>
          <Markdown startingLineNumber={start}>
            {"```" + extension + "\n" + file.file_content + "\n```"}
          </Markdown>
        </Box>
      )}
    </Box>
  );
};

function formatFileName(filePath: string, line1?: number, line2?: number): string {
  const name = filename(filePath);
  if (line1 && line2 && line1 !== 0 && line2 !== 0) {
    return `${name}:${line1}-${line2}`;
  }
  return name;
}

function extractEnrichmentDisplayName(filePath: string): string {
  const fileName = filename(filePath);

  // Memory files: 2025-12-26_230536_3fe00894_servicebobpy-is-a-standalone-fastapi.md
  // Extract the readable part after the hash
  const memoryMatch = fileName.match(/^\d{4}-\d{2}-\d{2}_\d{6}_[a-f0-9]+_(.+)\.md$/);
  if (memoryMatch) {
    return memoryMatch[1].replace(/-/g, " ");
  }

  // Trajectory files: UUID.json - show as "Conversation"
  const trajectoryMatch = fileName.match(/^[a-f0-9-]{36}\.json$/);
  if (trajectoryMatch) {
    return "Past conversation";
  }

  return fileName;
}

function extractProjectContextDisplayName(filePath: string): string {
  // For project context files, show relative path from project root
  // e.g., "/path/to/project/.vscode/settings.json" -> ".vscode/settings.json"
  // or "/path/to/project/AGENTS.md" -> "AGENTS.md"

  const parts = filePath.split("/");

  // Find common project markers and take path from there
  const markers = [".vscode", ".idea", ".cursor", ".windsurf", ".github", ".refact", ".zed", ".fleet", ".claude"];
  for (let i = 0; i < parts.length; i++) {
    if (markers.includes(parts[i])) {
      return parts.slice(i).join("/");
    }
  }

  // For instruction files at root, just show the filename
  const fileName = filename(filePath);
  const instructionFiles = ["AGENTS.md", "CLAUDE.md", "GEMINI.md", "REFACT.md", ".cursorrules", "global_rules.md", "copilot-instructions.md", ".aider.conf.yml"];
  if (instructionFiles.some(f => fileName.toLowerCase() === f.toLowerCase())) {
    return fileName;
  }

  // Fallback: show last 2 path components
  if (parts.length >= 2) {
    return parts.slice(-2).join("/");
  }

  return fileName;
}
