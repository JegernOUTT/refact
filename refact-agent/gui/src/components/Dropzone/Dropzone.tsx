import React, { createContext, isValidElement, useCallback } from "react";
import { Image, X } from "lucide-react";
import { DropzoneInputProps, FileRejection, useDropzone } from "react-dropzone";
import { useCapsForToolUse } from "../../hooks";
import { useAttachedImages } from "../../hooks/useAttachedImages";
import { useAttachedFiles } from "../ChatForm/useCheckBoxes";
import { TruncateLeft } from "../Text";
import { Button, Icon, IconButton, Tooltip } from "../ui";
import styles from "./Dropzone.module.css";

export const FileUploadContext = createContext<{
  open: () => void;

  getInputProps: (props?: DropzoneInputProps) => DropzoneInputProps;
}>({
  open: () => ({}),
  getInputProps: () => ({}),
});

export const DropzoneProvider: React.FC<
  React.PropsWithChildren<{ asChild?: boolean }>
> = ({ asChild, ...props }) => {
  const { setError, processAndInsertImages, processAndInsertTextFiles } =
    useAttachedImages();
  const { isMultimodalitySupportedForCurrentModel } = useCapsForToolUse();

  const onDrop = useCallback(
    (acceptedFiles: File[], fileRejections: FileRejection[]): void => {
      const imageFiles = acceptedFiles.filter(
        (f) => f.type === "image/jpeg" || f.type === "image/png",
      );
      const textFiles = acceptedFiles.filter(
        (f) => f.type !== "image/jpeg" && f.type !== "image/png",
      );

      if (imageFiles.length > 0) {
        if (!isMultimodalitySupportedForCurrentModel) {
          setError("Current model does not support images");
        } else {
          processAndInsertImages(imageFiles);
        }
      }

      if (textFiles.length > 0) {
        processAndInsertTextFiles(textFiles);
      }

      if (fileRejections.length) {
        const rejectedFileMessage = fileRejections.map((file) => {
          const err = file.errors.reduce<string>((acc, cur) => {
            return acc + `${cur.code} ${cur.message}\n`;
          }, "");
          return `could not attach ${file.file.name}: ${err}`;
        });
        setError(rejectedFileMessage.join("\n"));
      }
    },
    [
      processAndInsertImages,
      processAndInsertTextFiles,
      setError,
      isMultimodalitySupportedForCurrentModel,
    ],
  );

  const dropzone = useDropzone({
    disabled: false,
    noClick: true,
    noKeyboard: true,
    onDrop,
  });

  const rootProps = dropzone.getRootProps();
  const children = props.children;
  const root =
    asChild && isValidElement<{ className?: string }>(children) ? (
      React.cloneElement(children, {
        ...rootProps,
        ...children.props,
        className: [rootProps.className, children.props.className]
          .filter(Boolean)
          .join(" "),
      })
    ) : (
      <div {...rootProps} {...props} />
    );

  return (
    <FileUploadContext.Provider
      value={{
        open: dropzone.open,
        getInputProps: dropzone.getInputProps,
      }}
    >
      {root}
    </FileUploadContext.Provider>
  );
};

export const DropzoneConsumer = FileUploadContext.Consumer;

export const AttachImagesButton = () => {
  const attachFileOnClick = useCallback(
    (
      event: { preventDefault: () => void; stopPropagation: () => void },
      open: () => void,
    ) => {
      event.preventDefault();
      event.stopPropagation();
      open();
    },
    [],
  );

  return (
    <DropzoneConsumer>
      {({ open, getInputProps }) => {
        const inputProps = getInputProps();
        return (
          <>
            <input {...inputProps} className={styles.hiddenInput} />
            <Tooltip>
              <Tooltip.Trigger asChild>
                <IconButton
                  aria-label="Attach images"
                  disabled={inputProps.disabled}
                  icon={Image}
                  size="sm"
                  variant="plain"
                  onClick={(event) => {
                    attachFileOnClick(event, open);
                  }}
                />
              </Tooltip.Trigger>
              <Tooltip.Content>Attach images</Tooltip.Content>
            </Tooltip>
          </>
        );
      }}
    </DropzoneConsumer>
  );
};

type FileListProps = {
  attachedFiles: ReturnType<typeof useAttachedFiles>;
};

export const FileList: React.FC<FileListProps> = ({ attachedFiles }) => {
  const { images, removeImage } = useAttachedImages();
  if (images.length === 0 && attachedFiles.files.length === 0) return null;
  return (
    <div className={styles.fileList} data-testid="attached_file_list">
      {images.map((file, index) => {
        const key = `image-${file.name}-${index}`;
        return (
          <FileButton
            key={key}
            onClick={() => removeImage(index)}
            fileName={file.name}
          />
        );
      })}
      {attachedFiles.files.map((file, index) => {
        const key = `file-${file.path}-${index}`;
        return (
          <FileButton
            key={key}
            fileName={file.name}
            onClick={() => attachedFiles.removeFile(file)}
          />
        );
      })}
    </div>
  );
};

const FileButton: React.FC<{ fileName: string; onClick: () => void }> = ({
  fileName,
  onClick,
}) => {
  return (
    <Button
      className={styles.fileChip}
      size="sm"
      type="button"
      variant="soft"
      onClick={onClick}
    >
      <TruncateLeft wrap="wrap">{fileName}</TruncateLeft>
      <Icon className={styles.fileChipIcon} icon={X} size="sm" tone="muted" />
    </Button>
  );
};
