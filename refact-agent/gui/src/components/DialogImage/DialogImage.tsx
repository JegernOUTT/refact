import React, { useState } from "react";
import { Image } from "lucide-react";
import { Dialog, Icon } from "../ui";
import { ImageViewport } from "./ImageViewport";
import styles from "./DialogImage.module.css";

const SIZE_MAP = {
  "1": "24px",
  "2": "32px",
  "3": "40px",
  "4": "48px",
  "5": "56px",
  "6": "64px",
  "7": "72px",
  "8": "80px",
  "9": "96px",
  auto: "auto",
} as const;

export const DialogImage: React.FC<{
  src: string;
  size?: keyof typeof SIZE_MAP;
  fallback?: React.ReactNode;
  alt?: string;
}> = ({
  size = "8",
  fallback = <Icon icon={Image} size="lg" />,
  src,
  alt = "",
}) => {
  const [open, setOpen] = useState(false);
  const [dimensions, setDimensions] = useState({ width: 1, height: 1 });
  const thumbnailStyle = {
    "--dialog-image-size": SIZE_MAP[size],
  } as React.CSSProperties;

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <Dialog.Trigger asChild>
        <button
          type="button"
          className={styles.trigger}
          style={thumbnailStyle}
          aria-label={alt ? `Open image: ${alt}` : "Open image viewer"}
        >
          <img className={styles.thumbnail} src={src} alt={alt} />
          <span className={styles.fallback}>{fallback}</span>
        </button>
      </Dialog.Trigger>
      <Dialog.Content
        className={styles.content}
        maxWidth="90vw"
        maxHeight="90vh"
      >
        <Dialog.Title className={styles.srOnly}>
          {alt || "Image viewer"}
        </Dialog.Title>
        {open && (
          <ImageViewport width={dimensions.width} height={dimensions.height}>
            <img
              src={src}
              alt={alt}
              draggable={false}
              width={dimensions.width}
              height={dimensions.height}
              onLoad={(event) =>
                setDimensions({
                  width: event.currentTarget.naturalWidth || 1,
                  height: event.currentTarget.naturalHeight || 1,
                })
              }
            />
          </ImageViewport>
        )}
      </Dialog.Content>
    </Dialog>
  );
};
