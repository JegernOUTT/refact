import React from "react";
import { Text } from "@radix-ui/themes";
import { Eye, ShieldAlert } from "lucide-react";

import { Button } from "../../components/ui";
import type { PrivacyFileRecord } from "../../services/refact/privacy";
import styles from "./PrivacyChat.module.css";

type WithheldOutputCardProps = {
  exitCode: number | null | undefined;
  files: PrivacyFileRecord[];
  localOnlyOutput: string;
};

function readSummary(files: PrivacyFileRecord[]): string {
  if (files.length === 0) return "it read guarded files";
  const paths = files.map((file) => file.path);
  if (paths.length === 1) return `it read ${paths[0]}`;
  return `it read ${paths[0]} and ${paths.length - 1} more`;
}

export const WithheldOutputCard: React.FC<WithheldOutputCardProps> = ({
  exitCode,
  files,
  localOnlyOutput,
}) => {
  const [revealed, setRevealed] = React.useState(false);

  return (
    <section className={styles.card} data-testid="privacy-withheld-output-card">
      <div className={styles.cardHeader}>
        <ShieldAlert className={styles.cardIcon} aria-hidden="true" />
        <div className={styles.cardCopy}>
          <Text as="div" size="2" weight="medium">
            Ran, exit {exitCode ?? "unknown"} — output withheld,{" "}
            {readSummary(files)}.
          </Text>
          <Text className={styles.cardDescription} as="div" size="2">
            This model cannot receive that output.
          </Text>
        </div>
      </div>
      <div className={styles.actions}>
        <Button
          size="sm"
          variant="soft"
          leftIcon={Eye}
          aria-expanded={revealed}
          onClick={() => setRevealed((current) => !current)}
        >
          {revealed ? "Hide local output" : "Show me"}
        </Button>
      </div>
      {revealed && (
        <pre className={styles.localOutput} data-testid="privacy-local-output">
          {localOnlyOutput}
        </pre>
      )}
    </section>
  );
};
