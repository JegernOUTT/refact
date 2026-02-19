import React, { useState, useCallback } from "react";
import { FileTextIcon } from "@radix-ui/react-icons";
import { Box, Flex, Text } from "@radix-ui/themes";
import classNames from "classnames";
import { ChatContextFile } from "../../../services/refact/types";
import { useEventsBusForIDE } from "../../../hooks";
import { ShikiCodeBlock } from "../../Markdown";
import { useDelayedUnmount } from "../../shared/useDelayedUnmount";
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
  onOpenFile: (file: { file_path: string; line?: number }) => Promise<void>;
}

const ContextFileItem: React.FC<ContextFileItemProps> = ({
  file,
  onOpenFile,
}) => {
  const [isOpen, setIsOpen] = useState(false);
  const { shouldRender, isAnimatingOpen } = useDelayedUnmount(isOpen, 200);
  const displayName = formatFileName(file.file_name, file.line1, file.line2);
  const extension = getExtensionFromName(file.file_name);

  const handleFileClick = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      void onOpenFile({ file_path: file.file_name, line: file.line1 });
    },
    [onOpenFile, file.file_name, file.line1],
  );

  const handleToggle = useCallback(() => {
    setIsOpen((prev) => !prev);
  }, []);

  return (
    <div className={styles.item}>
      <Flex
        className={styles.header}
        align="center"
        gap="2"
        onClick={handleToggle}
      >
        <FileTextIcon className={styles.icon} />
        <Text size="1" className={styles.filename} onClick={handleFileClick}>
          {displayName}
        </Text>
      </Flex>

      {shouldRender && (
        <div
          className={classNames(
            styles.contentWrapper,
            isAnimatingOpen && styles.contentWrapperOpen,
          )}
        >
          <div className={styles.contentInner}>
            <Box className={styles.content}>
              <ShikiCodeBlock
                className={extension ? `language-${extension}` : undefined}
                showLineNumbers={false}
              >
                {file.file_content}
              </ShikiCodeBlock>
            </Box>
          </div>
        </div>
      )}
    </div>
  );
};

interface ContextFileListProps {
  files: ChatContextFile[];
}

export const ContextFileList: React.FC<ContextFileListProps> = ({ files }) => {
  const { queryPathThenOpenFile } = useEventsBusForIDE();

  if (files.length === 0) return null;

  return (
    <Flex direction="column" gap="1" className={styles.list}>
      {files.map((file, index) => (
        <ContextFileItem
          key={`${file.file_name}-${file.line1}-${file.line2}-${index}`}
          file={file}
          onOpenFile={queryPathThenOpenFile}
        />
      ))}
    </Flex>
  );
};

export default ContextFileList;
