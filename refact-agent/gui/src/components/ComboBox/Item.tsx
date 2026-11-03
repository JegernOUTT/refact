import React from "react";
import { ComboboxItem } from "@ariakit/react";
import classNames from "classnames";
import { KitBadge, kit } from "../ComposerKit";
import { TruncateLeft } from "../Text";
import type { CompletionDetail } from "../../services/refact/commands";
import styles from "./ComboBox.module.css";

export const Item: React.FC<{
  onClick: React.MouseEventHandler<HTMLDivElement>;
  value: string;
  detail?: CompletionDetail;
}> = ({ value, detail, onClick }) => {
  return (
    <ComboboxItem
      value={value}
      onClick={onClick}
      focusOnHover
      clickOnEnter={false}
      className={classNames(kit.row, styles.item)}
    >
      <span className={classNames(kit.rowLabelText, styles.name)}>
        <TruncateLeft>{value.trim()}</TruncateLeft>
      </span>
      {detail?.description ? (
        <span className={styles.description}>{detail.description}</span>
      ) : null}
      {detail?.kind === "skill" ? (
        <KitBadge tone="accent" className={styles.badge}>
          skill
        </KitBadge>
      ) : null}
    </ComboboxItem>
  );
};
