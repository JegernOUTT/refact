import React from "react";
import styles from "./FileUpload.module.css";

export type FileUploadProps = {
  onClick: (value: boolean) => void;
  fileName?: string;
  checked: boolean;
  disabled?: boolean;
};

export const FileUpload: React.FC<FileUploadProps> = ({
  onClick,
  fileName,
  checked,
  disabled,
}) => {
  return (
    <label className={styles.label}>
      <input
        checked={checked}
        className={styles.checkbox}
        disabled={disabled}
        type="checkbox"
        onChange={() => {
          onClick(!checked);
        }}
      />
      <span className={styles.text}>Attach {fileName ?? "a file"}</span>
    </label>
  );
};
