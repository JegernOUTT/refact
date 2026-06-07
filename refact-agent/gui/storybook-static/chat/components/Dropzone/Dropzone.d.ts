import { JSX } from 'react/jsx-runtime';
import React from "react";
import { DropzoneInputProps } from "react-dropzone";
import { useAttachedFiles } from "../ChatForm/useCheckBoxes";
export declare const FileUploadContext: React.Context<{
    open: () => void;
    getInputProps: (props?: DropzoneInputProps) => DropzoneInputProps;
}>;
export declare const DropzoneProvider: React.FC<React.PropsWithChildren<{
    asChild?: boolean;
}>>;
export declare const DropzoneConsumer: React.Consumer<{
    open: () => void;
    getInputProps: (props?: DropzoneInputProps) => DropzoneInputProps;
}>;
export declare const AttachImagesButton: () => JSX.Element;
type FileListProps = {
    attachedFiles: ReturnType<typeof useAttachedFiles>;
};
export declare const FileList: React.FC<FileListProps>;
export {};
