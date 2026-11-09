import { forwardRef, useId } from "react";
import type { LucideProps } from "lucide-react";
import styles from "./ActivityRail.module.css";

export const DaemonConsoleIcon = forwardRef<SVGSVGElement, LucideProps>(
  function DaemonConsoleIcon(props, ref) {
    const gradientId = useId();
    return (
      <svg
        {...props}
        ref={ref}
        viewBox="0 0 24 24"
        fill="none"
        aria-hidden="true"
        focusable="false"
      >
        <defs>
          <linearGradient
            id={gradientId}
            x1="2"
            y1="3"
            x2="22"
            y2="21"
            gradientUnits="userSpaceOnUse"
          >
            <stop className={styles.daemonWarm} offset="0" />
            <stop className={styles.daemonAccent} offset="0.5" />
            <stop className={styles.daemonCool} offset="1" />
          </linearGradient>
        </defs>
        <g
          stroke={`url(#${gradientId})`}
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M7 4H3v16h4M17 4h4v16h-4M11 4c-2 0-2 2-2 4s-1 3-2 4c1 1 2 2 2 4s0 4 2 4" />
          <path d="m14 9 3 3-3 3M11 12h6" />
          <circle cx="17" cy="12" r="2.5" fill="var(--rf-surface-1)" />
        </g>
      </svg>
    );
  },
);
