import React, { useCallback, useEffect, useState } from "react";
import { Check, Copy, X } from "lucide-react";
import { Dialog, IconButton, Tooltip } from "../ui";
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

type ExtensionColorKey =
  | "blue"
  | "orange"
  | "yellow"
  | "purple"
  | "pink"
  | "red"
  | "cyan"
  | "green"
  | "gray";

const EXTENSION_COLORS: Record<string, ExtensionColorKey> = {
  py: "blue",
  rs: "orange",
  js: "yellow",
  ts: "blue",
  tsx: "blue",
  jsx: "yellow",
  java: "orange",
  kt: "purple",
  cpp: "pink",
  c: "gray",
  h: "gray",
  go: "cyan",
  rb: "red",
  php: "purple",
  json: "gray",
  yaml: "red",
  yml: "red",
  toml: "orange",
  xml: "blue",
  html: "orange",
  css: "purple",
  scss: "pink",
  md: "blue",
  txt: "gray",
  env: "green",
  sh: "green",
  bash: "green",
  zsh: "green",
};

function getExtensionColor(ext: string): ExtensionColorKey {
  const color = EXTENSION_COLORS[ext.toLowerCase()] as
    | ExtensionColorKey
    | undefined;
  return color ?? "gray";
}

function getExtension(filename: string): string {
  if (filename.startsWith(".")) {
    return filename.slice(1).toUpperCase();
  }
  const parts = filename.split(".");
  if (parts.length > 1) {
    return parts[parts.length - 1].toUpperCase();
  }
  return "FILE";
}

function truncateFilename(filename: string, maxLength = 12): string {
  const basename = filename.split(/[/\\]/).pop() ?? filename;
  if (basename.length <= maxLength) return basename;

  const ext = basename.lastIndexOf(".");
  if (ext > 0) {
    const name = basename.substring(0, ext);
    const extension = basename.substring(ext);
    const availableLength = maxLength - extension.length - 2;
    if (availableLength > 0) {
      return name.substring(0, availableLength) + ".." + extension;
    }
  }
  return basename.substring(0, maxLength - 2) + "..";
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
    <div className={`${styles.tile} rf-enter`}>
      <Dialog>
        <Dialog.Trigger asChild>
          <button
            aria-label={`Preview ${name}`}
            className={styles.imageTrigger}
            type="button"
          >
            <img
              src={src}
              alt={name}
              className={styles.imageThumbnail}
              title={name}
            />
          </button>
        </Dialog.Trigger>
        <Dialog.Content maxWidth="800px">
          <img className={styles.previewImage} src={src} alt={name} />
        </Dialog.Content>
      </Dialog>
      {onRemove && (
        <IconButton
          aria-label="Remove image"
          className={styles.removeButton}
          icon={X}
          size="sm"
          type="button"
          variant="soft"
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
        />
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
  const ext = getExtension(name);
  const colorKey = getExtensionColor(ext.toLowerCase());
  const displayName = truncateFilename(name);
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
    <Tooltip>
      <Tooltip.Trigger asChild>
        <div
          className={`${styles.tile} ${styles.fileTile} rf-enter`}
          data-color={colorKey}
          tabIndex={0}
          onKeyDown={handleKeyDown}
          onClick={handleClick}
          title={`${name}${subtitle ? ` ${subtitle}` : ""}`}
          role="button"
          aria-label={`File: ${name}${subtitle ? ` ${subtitle}` : ""}`}
        >
          <span className={styles.extensionBadge}>.{ext}</span>
          <span className={styles.filename}>{displayName}</span>
          {subtitle && <span className={styles.subtitle}>{subtitle}</span>}
          <IconButton
            aria-label={copied ? "Copied!" : "Copy path"}
            className={styles.copyButton}
            icon={copied ? Check : Copy}
            size="sm"
            type="button"
            variant="plain"
            onClick={(e) => void handleCopy(e)}
          />
          {onRemove && (
            <IconButton
              aria-label="Remove"
              className={styles.removeButton}
              icon={X}
              size="sm"
              type="button"
              variant="soft"
              onClick={(e) => {
                e.stopPropagation();
                onRemove();
              }}
            />
          )}
        </div>
      </Tooltip.Trigger>
      <Tooltip.Content>{`${copyShortcut} to copy path`}</Tooltip.Content>
    </Tooltip>
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
    <Tooltip>
      <Tooltip.Trigger asChild>
        <div
          className={`${styles.tile} ${styles.plainTextTile} rf-enter`}
          tabIndex={0}
          onKeyDown={handleKeyDown}
          title={
            preview.length > 100 ? `${preview.substring(0, 100)}...` : preview
          }
          role="button"
          aria-label="Plain text content"
        >
          <span className={styles.extensionBadge}>TXT</span>
          <span className={styles.filename}>{label}</span>
          <IconButton
            aria-label={copied ? "Copied!" : "Copy content"}
            className={styles.copyButton}
            icon={copied ? Check : Copy}
            size="sm"
            type="button"
            variant="plain"
            onClick={(e) => void handleCopy(e)}
          />
        </div>
      </Tooltip.Trigger>
      <Tooltip.Content>
        {copied ? "Copied!" : `${copyShortcut} to copy`}
      </Tooltip.Content>
    </Tooltip>
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
