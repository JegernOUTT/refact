import React from "react";
import { Badge } from "../ui";
import type { CompletionDetail } from "../../services/refact/commands";
import styles from "./SlashCommandSuggestion.module.css";

type SlashCommandSuggestionProps = {
  name: string;
  detail?: CompletionDetail;
};

export const SlashCommandSuggestion: React.FC<SlashCommandSuggestionProps> = ({
  name,
  detail,
}) => (
  <div className={styles.suggestion}>
    <span className={styles.name}>{name}</span>
    {detail?.description && (
      <span className={styles.description}>{detail.description}</span>
    )}
    <Badge tone={detail?.kind === "skill" ? "accent" : "muted"} className={styles.badge}>
      {detail?.kind === "skill" ? "skill" : "cmd"}
    </Badge>
  </div>
);
