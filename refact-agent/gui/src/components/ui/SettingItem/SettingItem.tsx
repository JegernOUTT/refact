import classNames from "classnames";
import type React from "react";

import type { SaveStatusState } from "../Field";
import { SaveStatus } from "../Field";
import styles from "./SettingItem.module.css";

export type SettingItemLayout = "row" | "stack";

export interface SettingItemProps extends Omit<React.ComponentProps<"div">, "title"> {
  title: React.ReactNode;
  description?: React.ReactNode;
  control?: React.ReactNode;
  children?: React.ReactNode;
  layout?: SettingItemLayout;
  saveStatus?: SaveStatusState;
  saveStatusLabel?: string;
}

export function SettingItem({
  children,
  className,
  control,
  description,
  layout = "row",
  saveStatus = "idle",
  saveStatusLabel,
  title,
  ...props
}: SettingItemProps) {
  const content = control ?? children;

  return (
    <div {...props} className={classNames(styles.item, styles[layout], className)}>
      <div className={styles.copy}>
        <div className={styles.titleRow}>
          <h3 className={styles.title}>{title}</h3>
          <SaveStatus label={saveStatusLabel} state={saveStatus} />
        </div>
        {description ? <p className={styles.description}>{description}</p> : null}
      </div>
      {content ? <div className={styles.control}>{content}</div> : null}
    </div>
  );
}
