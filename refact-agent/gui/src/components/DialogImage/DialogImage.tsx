import React from "react";
import { Image } from "lucide-react";
import { Dialog, Icon } from "../ui";
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
} as const;

export const DialogImage: React.FC<{
  src: string;
  size?: keyof typeof SIZE_MAP;
  fallback?: React.ReactNode;
}> = ({ size = "8", fallback = <Icon icon={Image} size="lg" />, src }) => {
  const thumbnailStyle = {
    "--dialog-image-size": SIZE_MAP[size],
  } as React.CSSProperties;

  return (
    <Dialog>
      <Dialog.Trigger asChild>
        <button type="button" className={styles.trigger} style={thumbnailStyle}>
          <img className={styles.thumbnail} src={src} alt="" />
          <span className={styles.fallback}>{fallback}</span>
        </button>
      </Dialog.Trigger>
      <Dialog.Content maxWidth="800px">
        <img className={styles.image} src={src} alt="" />
      </Dialog.Content>
    </Dialog>
  );
};
