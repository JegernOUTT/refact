import React from "react";
export type FileUploadProps = {
    onClick: (value: boolean) => void;
    fileName?: string;
    checked: boolean;
    disabled?: boolean;
};
export declare const FileUpload: React.FC<FileUploadProps>;
