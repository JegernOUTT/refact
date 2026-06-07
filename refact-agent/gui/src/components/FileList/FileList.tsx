import React from "react";
import { File } from "lucide-react";
import { TruncateLeft } from "../Text";
import { Icon } from "../ui";
import type { ChatContextFile } from "../../services/refact";
import styles from "./file-list.module.css";

export type FileListProps = { files: ChatContextFile[] };
export const FileList: React.FC<FileListProps> = ({ files }) => {
  return (
    <div className={styles.list}>
      {files.map((file, i) => {
        const name = `${file.file_name}:${file.line1}-${file.line2}`;
        const key = `${name}--${i}`;
        return (
          <div key={key} title={file.file_content} className={styles.file}>
            <Icon icon={File} size="sm" />
            <TruncateLeft>{name}</TruncateLeft>
          </div>
        );
      })}
    </div>
  );
};
