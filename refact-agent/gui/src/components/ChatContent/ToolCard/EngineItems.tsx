import { useMemo, useState } from "react";
import classNames from "classnames";
import type { PythonEngineItem } from "../../../services/refact/types";
import { DiffBlock } from "./DiffBlock";
import { ClampedText } from "./PythonBlocks";
import {
  engineValueView,
  hasNoValue,
  isTextPair,
  textPairHunks,
} from "./engineChanges";
import { plural } from "./pythonDisplay";
import { shortText } from "./safeText";
import { ShortText } from "./ShortText";
import styles from "./EngineItems.module.css";

const TEXT_PAIR_CHANGES = 20;

function Side({
  value,
  side,
  lines,
  inline = false,
}: {
  value: unknown;
  side: "old" | "new";
  lines: number;
  inline?: boolean;
}) {
  const view = engineValueView(value, side);
  if (view.kind === "absent" && inline)
    return <span className={styles.absent}>{view.text}</span>;
  if (view.kind === "inline" && inline)
    return (
      <code className={styles.inline} title={view.text}>
        <bdi>{view.text}</bdi>
      </code>
    );
  return (
    <div className={styles.block}>
      <span className={styles.sideLabel}>{side === "old" ? "Was" : "Now"}</span>
      {view.kind === "absent" ? (
        <span className={styles.absent}>{view.text}</span>
      ) : view.kind === "inline" ? (
        <code className={styles.inline}>
          <bdi>{view.text}</bdi>
        </code>
      ) : (
        <ClampedText text={view.text} lines={lines} />
      )}
    </div>
  );
}

function TextPair({
  item,
  label,
}: {
  item: PythonEngineItem & { old: string; new: string };
  label: string;
}) {
  const hunks = useMemo(
    () => textPairHunks(label, item.old, item.new),
    [item.new, item.old, label],
  );
  const [all, setAll] = useState(false);
  if (hunks.length === 0)
    return <span className={styles.note}>No difference in the text</span>;
  const shown = all ? hunks : hunks.slice(0, TEXT_PAIR_CHANGES);
  return (
    <div className={styles.diff} aria-label={`Changes to ${label}`}>
      {shown.map((hunk, index) => (
        <DiffBlock
          key={index}
          diff={hunk}
          fileName={label}
          displayFileName={label}
        />
      ))}
      {shown.length < hunks.length && (
        <button
          type="button"
          className={styles.more}
          onClick={() => setAll(true)}
        >
          Show {plural(hunks.length - shown.length, "more change")}
        </button>
      )}
    </div>
  );
}

/** One `label: old → new` item; long or structured sides become bounded blocks. */
export function EngineItem({
  item,
  dense = false,
}: {
  item: PythonEngineItem;
  dense?: boolean;
}) {
  const lines = dense ? 4 : 6;
  const label = useMemo(() => shortText(item.label), [item.label]);
  const old = engineValueView(item.old, "old");
  const next = engineValueView(item.new, "new");
  const empty = hasNoValue(item);
  const inline = old.kind !== "block" && next.kind !== "block";
  const pair = !dense && isTextPair(item);
  const labelNode = <ShortText text={item.label} className={styles.label} />;
  return (
    <li
      className={classNames(styles.item, dense && styles.dense)}
      data-testid="engine-item"
    >
      {empty ? (
        <span className={styles.line}>
          {labelNode}
          <span className={styles.absent}>no value given</span>
        </span>
      ) : inline ? (
        <span className={styles.line}>
          <span className={styles.labelGroup}>
            {labelNode}
            <span className={styles.label}>:</span>
          </span>
          <Side value={item.old} side="old" lines={lines} inline />
          <span className={styles.arrow} aria-hidden>
            →
          </span>
          <span className={styles.srOnly}>changes to</span>
          <Side value={item.new} side="new" lines={lines} inline />
        </span>
      ) : (
        <>
          {labelNode}
          {pair ? (
            <TextPair item={item} label={label.shown} />
          ) : (
            <div className={styles.blocks}>
              <Side value={item.old} side="old" lines={lines} />
              <Side value={item.new} side="new" lines={lines} />
            </div>
          )}
        </>
      )}
      {item.truncated && (
        <span className={styles.note}>Value cut by the engine</span>
      )}
    </li>
  );
}

export function EngineItems({
  items,
  dense = false,
}: {
  items: readonly PythonEngineItem[];
  dense?: boolean;
}) {
  if (items.length === 0) return null;
  return (
    <ul className={styles.items}>
      {items.map((item, index) => (
        <EngineItem key={index} item={item} dense={dense} />
      ))}
    </ul>
  );
}
