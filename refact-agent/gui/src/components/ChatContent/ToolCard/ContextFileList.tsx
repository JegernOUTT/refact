import React from "react";
import { ChatContextFile } from "../../../services/refact/types";
import { useOpenFileInApp } from "../../../hooks/useOpenFileInApp";
import { ShikiCodeBlock } from "../../Markdown";
import { TranscriptClamp } from "../TranscriptItem";
import { contextFileLineCount } from "./readToolPaths";
import styles from "./ContextFileList.module.css";

function formatFileName(file: ChatContextFile): string {
  const parts = file.file_name.split("/");
  const name = parts[parts.length - 1] || file.file_name;
  if (file.line1 && file.line2) return `${name}:${file.line1}-${file.line2}`;
  return name;
}

function extensionOf(path: string): string {
  const name = path.split("/").pop() ?? path;
  const dot = name.lastIndexOf(".");
  return dot === -1 ? "" : name.substring(dot + 1);
}

interface ContextFileListProps {
  files: ChatContextFile[];
}

export const ContextFileList: React.FC<ContextFileListProps> = ({ files }) => {
  const { canOpen, openFile } = useOpenFileInApp();

  if (files.length === 0) return null;

  return (
    <ul className={styles.list} data-testid="context-file-list">
      {files.map((file, index) => {
        const displayName = formatFileName(file);
        const extension = extensionOf(file.file_name);
        return (
          <li
            className={styles.item}
            key={`${file.file_name}-${file.line1}-${file.line2}-${index}`}
          >
            <div className={styles.header}>
              {canOpen ? (
                <button
                  className={styles.filename}
                  title={file.file_name}
                  type="button"
                  onClick={() =>
                    openFile({ path: file.file_name, line: file.line1 })
                  }
                >
                  {displayName}
                </button>
              ) : (
                <span className={styles.filenamePlain} title={file.file_name}>
                  {displayName}
                </span>
              )}
              <span className={styles.lines}>
                {contextFileLineCount(file)} lines
              </span>
            </div>
            {file.file_content ? (
              <TranscriptClamp lineCount={contextFileLineCount(file)}>
                <ShikiCodeBlock
                  className={extension ? `language-${extension}` : undefined}
                  showLineNumbers={false}
                >
                  {file.file_content}
                </ShikiCodeBlock>
              </TranscriptClamp>
            ) : null}
          </li>
        );
      })}
    </ul>
  );
};

export default ContextFileList;
