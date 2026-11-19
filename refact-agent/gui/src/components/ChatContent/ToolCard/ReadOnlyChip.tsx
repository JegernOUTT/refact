import { Lock } from "lucide-react";
import { Badge, Icon, Tooltip } from "../../ui";
import styles from "./ReadOnlyChip.module.css";

export const READ_ONLY_NOTE =
  "The script could write files only in the artifacts folder of its workspace: the chat had no tool that edits files or runs commands. Tools the script called follow their own rules.";

export function ReadOnlyChip() {
  return (
    <Tooltip content={READ_ONLY_NOTE}>
      <Badge tone="muted" size="sm" tabIndex={0} className={styles.chip}>
        <Icon icon={Lock} size="sm" className={styles.icon} />
        <span className={styles.label}>Read-only</span>
      </Badge>
    </Tooltip>
  );
}
