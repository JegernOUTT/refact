import React, { useCallback, useEffect, useState } from "react";
import { Check, Copy, FileText, X } from "lucide-react";
import { Dialog } from "../ui";
import { KitIconButton, KitTooltip } from "../ComposerKit";
import styles from "./AttachmentTile.module.css";

const isMac =
  typeof navigator !== "undefined" &&
  /Mac|iPod|iPhone|iPad/.test(navigator.platform);
const copyShortcut = isMac ? "⌘C" : "Ctrl+C";

async function copyToClipboard(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    try {
      const textArea = document.createElement("textarea");
      textArea.value = text;
      textArea.style.position = "fixed";
      textArea.style.left = "-9999px";
      textArea.style.top = "-9999px";
      document.body.appendChild(textArea);
      textArea.focus();
      textArea.select();
      const success = document.execCommand("copy");
      document.body.removeChild(textArea);
      return success;
    } catch {
      return false;
    }
  }
}

function basename(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

export type AttachmentTileProps =
  | {
      kind: "image";
      id: string;
      name: string;
      src: string;
      onRemove?: () => void;
    }
  | {
      kind: "file";
      id: string;
      name: string;
      copyText: string;
      subtitle?: string;
      onRemove?: () => void;
      onOpen?: () => void | Promise<void>;
    }
  | {
      kind: "plain-text";
      id: string;
      label: string;
      preview: string;
      copyText: string;
    };

const ImageTile: React.FC<{
  src: string;
  name: string;
  onRemove?: () => void;
}> = ({ src, name, onRemove }) => {
  return (
    <div className={`${styles.chip} rf-enter`}>
      <Dialog>
        <Dialog.Trigger asChild>
          <button
            aria-label={`Preview ${name}`}
            className={styles.imageTrigger}
            type="button"
          >
            <img src={src} alt="" className={styles.imageThumbnail} />
            <span className={styles.filename}>{name}</span>
          </button>
        </Dialog.Trigger>
        <Dialog.Content maxWidth="800px">
          <img className={styles.previewImage} src={src} alt={name} />
        </Dialog.Content>
      </Dialog>
      {onRemove && (
        <span className={styles.actions}>
          <KitIconButton
            aria-label="Remove image"
            icon={X}
            onClick={(e) => {
              e.stopPropagation();
              onRemove();
            }}
          />
        </span>
      )}
    </div>
  );
};

const FileTile: React.FC<{
  name: string;
  copyText: string;
  subtitle?: string;
  onRemove?: () => void;
  onOpen?: () => void | Promise<void>;
}> = ({ name, copyText, subtitle, onRemove, onOpen }) => {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (copied) {
      const timer = setTimeout(() => setCopied(false), 1500);
      return () => clearTimeout(timer);
    }
  }, [copied]);

  const handleCopy = useCallback(
    async (e: React.MouseEvent | React.KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const success = await copyToClipboard(copyText);
      if (success) {
        setCopied(true);
      }
    },
    [copyText],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "c") {
        void handleCopy(e);
      }
      if (e.key === "Enter" && onOpen) {
        e.preventDefault();
        void onOpen();
      }
    },
    [handleCopy, onOpen],
  );

  const handleClick = useCallback(() => {
    if (onOpen) {
      void onOpen();
    }
  }, [onOpen]);

  return (
    <KitTooltip label={`${copyShortcut} to copy path`}>
      <div
        className={`${styles.chip} rf-enter`}
        data-clickable={onOpen ? "true" : undefined}
        tabIndex={0}
        onKeyDown={handleKeyDown}
        onClick={handleClick}
        title={`${name}${subtitle ? ` ${subtitle}` : ""}`}
        role="button"
        aria-label={`File: ${name}${subtitle ? ` ${subtitle}` : ""}`}
      >
        <FileText aria-hidden="true" className={styles.icon} />
        <span className={styles.filename}>{basename(name)}</span>
        {subtitle && <span className={styles.meta}>{subtitle}</span>}
        <span className={styles.actions}>
          <KitIconButton
            aria-label={copied ? "Copied!" : "Copy path"}
            icon={copied ? Check : Copy}
            onClick={(e) => void handleCopy(e)}
          />
          {onRemove && (
            <KitIconButton
              aria-label="Remove"
              icon={X}
              onClick={(e) => {
                e.stopPropagation();
                onRemove();
              }}
            />
          )}
        </span>
      </div>
    </KitTooltip>
  );
};

const PlainTextTile: React.FC<{
  label: string;
  preview: string;
  copyText: string;
}> = ({ label, preview, copyText }) => {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (copied) {
      const timer = setTimeout(() => setCopied(false), 1500);
      return () => clearTimeout(timer);
    }
  }, [copied]);

  const handleCopy = useCallback(
    async (e: React.MouseEvent | React.KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const success = await copyToClipboard(copyText);
      if (success) {
        setCopied(true);
      }
    },
    [copyText],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "c") {
        void handleCopy(e);
      }
    },
    [handleCopy],
  );

  return (
    <KitTooltip label={copied ? "Copied!" : `${copyShortcut} to copy`}>
      <div
        className={`${styles.chip} rf-enter`}
        tabIndex={0}
        onKeyDown={handleKeyDown}
        title={
          preview.length > 100 ? `${preview.substring(0, 100)}...` : preview
        }
        role="button"
        aria-label="Plain text content"
      >
        <FileText aria-hidden="true" className={styles.icon} />
        <span className={styles.filename}>{label}</span>
        <span className={styles.actions}>
          <KitIconButton
            aria-label={copied ? "Copied!" : "Copy content"}
            icon={copied ? Check : Copy}
            onClick={(e) => void handleCopy(e)}
          />
        </span>
      </div>
    </KitTooltip>
  );
};

export const AttachmentTile: React.FC<AttachmentTileProps> = (props) => {
  switch (props.kind) {
    case "image":
      return (
        <ImageTile
          src={props.src}
          name={props.name}
          onRemove={props.onRemove}
        />
      );
    case "file":
      return (
        <FileTile
          name={props.name}
          copyText={props.copyText}
          subtitle={props.subtitle}
          onRemove={props.onRemove}
          onOpen={props.onOpen}
        />
      );
    case "plain-text":
      return (
        <PlainTextTile
          label={props.label}
          preview={props.preview}
          copyText={props.copyText}
        />
      );
  }
};
