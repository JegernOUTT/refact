import type { ReactNode } from "react";
import styles from "./ReviewHeader.module.css";

export type ReviewHeaderProps = {
  stackControls?: ReactNode;
  identity: ReactNode;
  reviewControls?: ReactNode;
  status?: ReactNode;
  className?: string;
};

export function ReviewHeader({
  stackControls,
  identity,
  reviewControls,
  status,
  className,
}: ReviewHeaderProps) {
  return (
    <header className={[styles.header, className].filter(Boolean).join(" ")}>
      {stackControls && <div className={styles.controls}>{stackControls}</div>}
      {identity && <div className={styles.identity}>{identity}</div>}
      {status && <div className={styles.status}>{status}</div>}
      {reviewControls && (
        <div className={styles.controls}>{reviewControls}</div>
      )}
    </header>
  );
}
