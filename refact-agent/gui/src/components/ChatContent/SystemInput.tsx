import React from "react";
import { Markdown } from "../Markdown";

import styles from "./ChatContent.module.css";

type ChatInputProps = {
  children: string;
};

export const SystemInput: React.FC<ChatInputProps> = (props) => {
  return (
    <div className={styles.systemInput}>
      <Markdown>{props.children}</Markdown>
    </div>
  );
};
