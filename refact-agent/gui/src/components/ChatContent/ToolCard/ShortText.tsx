import { useMemo, useState } from "react";
import { shortText } from "./safeText";
import styles from "./EngineItems.module.css";

/** A cut, safe text with a toggle to show it in full. */
export function ShortText({
  text,
  className,
  max,
}: {
  text: string;
  className?: string;
  max?: number;
}) {
  const [open, setOpen] = useState(false);
  const { shown, full, cut } = useMemo(() => shortText(text, max), [max, text]);
  if (!cut) return <span className={className}>{full}</span>;
  return (
    <span className={className}>
      <span title={open ? undefined : full}>{open ? full : shown}</span>{" "}
      <button
        type="button"
        className={styles.more}
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        {open ? "Show less" : `Show all ${Array.from(full).length} characters`}
      </button>
    </span>
  );
}
