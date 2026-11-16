import { CircleHelp, ShieldAlert, Trash2 } from "lucide-react";
import { Badge, Icon } from "../../ui";
import type { PythonEngineChange } from "../../../services/refact/types";
import { EngineItems } from "./EngineItems";
import { ENGINE_CLASS_LABEL, sentenceStart } from "./engineChanges";
import { ShortText } from "./ShortText";
import { shortText, visibleText } from "./safeText";
import styles from "./EngineChangesSection.module.css";

function ChangeRow({ change }: { change: PythonEngineChange }) {
  const careful = change.class !== "change";
  const unconfirmed = change.outcome === "unknown";
  return (
    <li
      className={styles.change}
      data-class={change.class}
      data-outcome={change.outcome}
    >
      <div className={styles.head}>
        {unconfirmed && (
          <span className={styles.outcome}>
            <Icon icon={CircleHelp} size="sm" tone="muted" />
            Outcome unknown
          </span>
        )}
        {careful && (
          <span className={styles.marker} data-class={change.class}>
            <Icon
              icon={change.class === "destructive" ? Trash2 : ShieldAlert}
              size="sm"
              tone={change.class === "destructive" ? "danger" : "warning"}
            />
            {ENGINE_CLASS_LABEL[change.class]}
          </span>
        )}
        <ShortText
          text={sentenceStart(change.title)}
          className={styles.title}
        />
        {change.scope && (
          <Badge tone="default" size="sm" title={visibleText(change.scope)}>
            {shortText(change.scope, 60).shown}
          </Badge>
        )}
        <ShortText text={change.function} className={styles.function} />
      </div>
      {unconfirmed && (
        <p className={styles.note}>
          The engine could not confirm whether this change was applied.
        </p>
      )}
      <EngineItems items={change.items} dense />
    </li>
  );
}

/** Applied engine changes of a run: one compact row each, bounded in height. */
export function EngineChangesSection({
  changes,
  total,
}: {
  changes: readonly PythonEngineChange[];
  total?: number;
}) {
  const count = Math.max(total ?? 0, changes.length);
  return (
    <section className={styles.section} aria-label="Engine changes">
      <span className={styles.label}>Engine changes · {count}</span>
      <ul
        className={styles.list}
        tabIndex={changes.length > 3 ? 0 : undefined}
        aria-label="Engine changes list"
      >
        {changes.map((change) => (
          <ChangeRow key={change.seq} change={change} />
        ))}
      </ul>
      {count > changes.length && (
        <p className={styles.note}>
          Showing {changes.length} of {count}
        </p>
      )}
    </section>
  );
}
