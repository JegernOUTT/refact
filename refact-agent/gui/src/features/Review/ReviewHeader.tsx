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
    <header
      className={[styles.header, className].filter(Boolean).join(" ")}
      onWheel={({ currentTarget, target, deltaX, deltaY }) => {
        if (
          deltaX ||
          !(target instanceof Node) ||
          !currentTarget.contains(target) ||
          getComputedStyle(currentTarget).overflowX !== "auto"
        )
          return;
        currentTarget.scrollLeft += deltaY;
      }}
      onFocusCapture={({ currentTarget, target }) => {
        if (
          !currentTarget.contains(target) ||
          getComputedStyle(currentTarget).overflowX !== "auto"
        )
          return;
        const header = currentTarget.getBoundingClientRect();
        const control = (
          target.closest("label") ?? target
        ).getBoundingClientRect();
        if (control.right > header.right) {
          currentTarget.scrollLeft += control.right - header.right;
        } else if (control.left < header.left) {
          currentTarget.scrollLeft += control.left - header.left;
        }
      }}
    >
      {stackControls && <div className={styles.controls}>{stackControls}</div>}
      {identity && (
        <div className={styles.identitySlot}>
          <div className={styles.identity}>{identity}</div>
        </div>
      )}
      {status && <div className={styles.status}>{status}</div>}
      {reviewControls && (
        <div className={styles.controls}>{reviewControls}</div>
      )}
    </header>
  );
}
