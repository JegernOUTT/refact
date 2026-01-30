import React from "react";
import { Flex, Box, IconButton, Text, Dialog } from "@radix-ui/themes";
import { Cross1Icon, FileIcon } from "@radix-ui/react-icons";
import { useAttachedImages } from "../../hooks/useAttachedImages";
import { useAttachedFiles } from "./useCheckBoxes";
import styles from "./AttachmentsPreview.module.css";

type AttachmentsPreviewProps = {
  attachedFiles: ReturnType<typeof useAttachedFiles>;
};

function getFileExtension(filename: string): string {
  const parts = filename.split(".");
  if (parts.length > 1) {
    return parts[parts.length - 1].toUpperCase();
  }
  return "FILE";
}

function truncateFilename(filename: string, maxLength = 20): string {
  if (filename.length <= maxLength) return filename;
  const ext = filename.lastIndexOf(".");
  if (ext > 0) {
    const name = filename.substring(0, ext);
    const extension = filename.substring(ext);
    const availableLength = maxLength - extension.length - 3;
    if (availableLength > 0) {
      return name.substring(0, availableLength) + "..." + extension;
    }
  }
  return filename.substring(0, maxLength - 3) + "...";
}

const ImageThumbnail: React.FC<{
  src: string;
  name: string;
  onRemove: () => void;
}> = ({ src, name, onRemove }) => {
  return (
    <Box className={styles.thumbnailContainer}>
      <Dialog.Root>
        <Dialog.Trigger>
          <img
            src={src}
            alt={name}
            className={styles.thumbnail}
            title={name}
            style={{ cursor: "zoom-in" }}
          />
        </Dialog.Trigger>
        <Dialog.Content maxWidth="800px">
          <img style={{ objectFit: "contain", width: "100%" }} src={src} alt={name} />
        </Dialog.Content>
      </Dialog.Root>
      <IconButton
        type="button"
        size="1"
        variant="solid"
        color="gray"
        className={styles.removeButton}
        onClick={(e) => {
          e.stopPropagation();
          onRemove();
        }}
      >
        <Cross1Icon width={10} height={10} />
      </IconButton>
    </Box>
  );
};

const FileCard: React.FC<{
  name: string;
  onRemove: () => void;
}> = ({ name, onRemove }) => {
  const extension = getFileExtension(name);
  const displayName = truncateFilename(name);

  return (
    <Box className={styles.fileCard}>
      <Flex align="center" gap="1" className={styles.fileCardContent}>
        <FileIcon width={12} height={12} />
        <Text size="1" className={styles.fileExtension}>
          {extension}
        </Text>
        <Text size="1" className={styles.fileName} title={name}>
          {displayName}
        </Text>
      </Flex>
      <IconButton
        type="button"
        size="1"
        variant="ghost"
        color="gray"
        className={styles.fileRemoveButton}
        onClick={onRemove}
      >
        <Cross1Icon width={8} height={8} />
      </IconButton>
    </Box>
  );
};

export const AttachmentsPreview: React.FC<AttachmentsPreviewProps> = ({
  attachedFiles,
}) => {
  const { images, removeImage, textFiles, removeTextFile } = useAttachedImages();

  if (images.length === 0 && attachedFiles.files.length === 0 && textFiles.length === 0) {
    return null;
  }

  return (
    <Flex
      wrap="wrap"
      gap="2"
      className={styles.container}
      data-testid="attachments-preview"
    >
      {images.map((image, index) => {
        if (typeof image.content !== "string") return null;
        return (
          <ImageThumbnail
            key={`image-${image.name}-${index}`}
            src={image.content}
            name={image.name}
            onRemove={() => removeImage(index)}
          />
        );
      })}
      {textFiles.map((file, index) => (
        <FileCard
          key={`textfile-${file.name}-${index}`}
          name={file.name}
          onRemove={() => removeTextFile(index)}
        />
      ))}
      {attachedFiles.files.map((file, index) => (
        <FileCard
          key={`file-${file.path}-${index}`}
          name={file.name}
          onRemove={() => attachedFiles.removeFile(file)}
        />
      ))}
    </Flex>
  );
};
