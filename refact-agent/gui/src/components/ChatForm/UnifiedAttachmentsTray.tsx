import React, { useMemo } from "react";
import { Flex } from "@radix-ui/themes";
import { AttachmentTile, AttachmentTileProps } from "./AttachmentTile";
import { useAttachedImages } from "../../hooks/useAttachedImages";
import { useAttachedFiles } from "./useCheckBoxes";
import { ChatContextFile } from "../../services/refact";
import styles from "./UnifiedAttachmentsTray.module.css";
import type { ManualPreviewItem } from "../../features/Chat/Thread/types";

type UnifiedAttachmentsTrayProps = {
  attachedFiles: ReturnType<typeof useAttachedFiles>;
  previewFiles?: (ChatContextFile | string)[];
  manualPreviewItems?: ManualPreviewItem[];
  onRemoveManualPreviewItem?: (index: number) => void;
  onOpenFile?: (file: {
    file_path: string;
    line?: number;
  }) => void | Promise<void>;
};

function getFilename(path: string): string {
  const parts = path.split(/[/\\]/);
  return parts[parts.length - 1] || path;
}

function normalizePath(path: string): string {
  return path
    .trim()
    .replace(/:\d+(-\d+)?$/, "")
    .replace(/\\/g, "/");
}

function samePath(left: string, right: string): boolean {
  const a = normalizePath(left);
  const b = normalizePath(right);
  if (!a || !b) return false;
  if (a === b) return true;
  return a.endsWith(`/${b}`) || b.endsWith(`/${a}`);
}

function formatLineRange(
  line1: number | null | undefined,
  line2: number | null | undefined,
): string {
  const hasLine1 = typeof line1 === "number" && line1 > 0;
  const hasLine2 = typeof line2 === "number" && line2 > 0;
  if (hasLine1 && hasLine2) return `:${line1}-${line2}`;
  if (hasLine1) return `:${line1}`;
  return "";
}

function normalizeLine(line: number | null | undefined): number | undefined {
  return typeof line === "number" && line > 0 ? line : undefined;
}

export const UnifiedAttachmentsTray: React.FC<UnifiedAttachmentsTrayProps> = ({
  attachedFiles,
  previewFiles,
  manualPreviewItems,
  onRemoveManualPreviewItem,
  onOpenFile,
}) => {
  const { images, removeImage, textFiles, removeTextFile } =
    useAttachedImages();

  const items = useMemo(() => {
    const result: AttachmentTileProps[] = [];
    const addedFilePaths: string[] = [];

    manualPreviewItems?.forEach((item, index) => {
      result.push({
        kind: "file",
        id: `manual-preview-${item.context_file.file_name}-${index}`,
        name: item.label,
        subtitle: item.kind,
        copyText: item.context_file.file_name,
        onRemove: onRemoveManualPreviewItem
          ? () => onRemoveManualPreviewItem(index)
          : undefined,
      });
      addedFilePaths.push(item.context_file.file_name);
    });

    images.forEach((image, index) => {
      if (typeof image.content === "string") {
        result.push({
          kind: "image",
          id: `image-${image.name}-${index}`,
          name: image.name,
          src: image.content,
          onRemove: () => removeImage(index),
        });
      }
    });

    textFiles.forEach((file, index) => {
      result.push({
        kind: "file",
        id: `textfile-${file.name}-${index}`,
        name: file.name,
        copyText: file.name,
        onRemove: () => removeTextFile(index),
      });
    });

    attachedFiles.files.forEach((file, index) => {
      const lineRange = formatLineRange(file.line1, file.line2);
      addedFilePaths.push(file.path);
      result.push({
        kind: "file",
        id: `attached-${file.path}-${index}`,
        name: getFilename(file.path),
        copyText: `@file ${file.path}${lineRange}`,
        subtitle: lineRange || undefined,
        onRemove: () => attachedFiles.removeFile(file),
        onOpen: onOpenFile
          ? () =>
              onOpenFile({
                file_path: file.path,
                line: normalizeLine(file.line1),
              })
          : undefined,
      });
    });

    if (previewFiles) {
      previewFiles.forEach((file, index) => {
        if (typeof file === "string") {
          result.push({
            kind: "plain-text",
            id: `plain-text-${index}`,
            label: "plain text",
            preview: file,
            copyText: file,
          });
        } else {
          if (addedFilePaths.some((path) => samePath(path, file.file_name))) {
            return;
          }
          addedFilePaths.push(file.file_name);
          const lineRange = formatLineRange(file.line1, file.line2);
          result.push({
            kind: "file",
            id: `preview-${file.file_name}-${index}`,
            name: getFilename(file.file_name),
            copyText: `@file ${file.file_name}${lineRange}`,
            subtitle: lineRange || undefined,
            onOpen: onOpenFile
              ? () =>
                  onOpenFile({
                    file_path: file.file_name,
                    line: normalizeLine(file.line1),
                  })
              : undefined,
          });
        }
      });
    }

    return result;
  }, [
    images,
    textFiles,
    attachedFiles,
    previewFiles,
    manualPreviewItems,
    onRemoveManualPreviewItem,
    removeImage,
    removeTextFile,
    onOpenFile,
  ]);

  if (items.length === 0) {
    return null;
  }

  return (
    <Flex wrap="wrap" gap="2" className={styles.tray}>
      {items.map((item) => (
        <AttachmentTile key={item.id} {...item} />
      ))}
    </Flex>
  );
};
