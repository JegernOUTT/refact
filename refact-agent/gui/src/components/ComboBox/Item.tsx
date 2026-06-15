import React from "react";
import { ComboboxItem } from "@ariakit/react";
import styles from "./ComboBox.module.css";

export const Item: React.FC<{
  onClick: React.MouseEventHandler<HTMLDivElement>;
  value: string;
  children: React.ReactNode;
}> = ({ children, value, onClick }) => {
  return (
    <ComboboxItem
      value={value}
      onClick={onClick}
      focusOnHover
      clickOnEnter={false}
      className={styles.item}
    >
      <span className={styles.combobox__item}>{children}</span>
    </ComboboxItem>
  );
};
