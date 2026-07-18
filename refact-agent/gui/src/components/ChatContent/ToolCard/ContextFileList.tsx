import { FileText } from "lucide-react";
import React, { useState, useCallback } from "react";
import { Box, Text } from "@radix-ui/themes";
import { ChatContextFile } from "../../../services/refact/types";
import { useOpenFileInApp } from "../../../hooks";
import { ShikiCodeBlock } from "../../Markdown";
import { AnimatedCollapsible } from "../shared/AnimatedCollapsible";
import styles from "./ContextFileList.module.css";

function filename(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1] || path;
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

function getExtensionFromName(name: string): string {
  const dot = name.lastIndexOf(".");
  if (dot === -1) return "";
  return name.substring(dot + 1).replace(/:\d*-\d*/, "");
}

interface ContextFileItemProps {
  file: ChatContextFile;
  onOpenFile: (file: { path: string; line?: number }) => void;
  canOpen: boolean;
}

const ContextFileItem: React.FC<ContextFileItemProps> = ({
  file,
  onOpenFile,
  canOpen,
}) => {
  const [isOpen, setIsOpen] = useState(false);
  const displayName = formatFileName(file.file_name, file.line1, file.line2);
  const extension = getExtensionFromName(file.file_name);

  const handleFileClick = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      onOpenFile({ path: file.file_name, line: file.line1 });
    },
    [onOpenFile, file.file_name, file.line1],
  );

  const handleOpenChange = useCallback((open: boolean) => {
    setIsOpen(open);
  }, []);

  return (
    <AnimatedCollapsible
      className={styles.item}
      header={
        <Text
          size="1"
          className={canOpen ? styles.filename : styles.filenamePlain}
          onClick={canOpen ? handleFileClick : undefined}
        >
          {displayName}
        </Text>
      }
      icon={<FileText className={styles.icon} />}
      open={isOpen}
      onOpenChange={handleOpenChange}
      variant="compact"
    >
      <Box className={styles.content}>
        <ShikiCodeBlock
          className={extension ? `language-${extension}` : undefined}
          showLineNumbers={false}
        >
          {file.file_content}
        </ShikiCodeBlock>
      </Box>
    </AnimatedCollapsible>
  );
};

interface ContextFileListProps {
  files: ChatContextFile[];
}

export const ContextFileList: React.FC<ContextFileListProps> = ({ files }) => {
  const { canOpen, openFile } = useOpenFileInApp();

  if (files.length === 0) return null;

  return (
    <div className={styles.list}>
      {files.map((file, index) => (
        <ContextFileItem
          key={`${file.file_name}-${file.line1}-${file.line2}-${index}`}
          file={file}
          onOpenFile={openFile}
          canOpen={canOpen}
        />
      ))}
    </div>
  );
};

export default ContextFileList;
