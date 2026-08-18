import { Flex } from "@radix-ui/themes";
import classNames from "classnames";
import { CircleAlert, Globe, TriangleAlert } from "lucide-react";

import type { BrowserConsoleCounts } from "../../../services/refact/browser";
import { Badge, Icon } from "../../ui";
import styles from "./PageHeader.module.css";

export interface PageHeaderProps {
  url?: string | null;
  title?: string | null;
  status?: number | null;
  console: BrowserConsoleCounts;
  consoleOpen?: boolean;
  onToggleConsole?: () => void;
}

function statusDetails(status: number): {
  tone: "muted" | "danger";
  state: string;
} {
  if (status >= 400) return { tone: "danger", state: "error" };
  return { tone: "muted", state: "neutral" };
}

function countsLabel(counts: BrowserConsoleCounts): string {
  const parts: string[] = [];
  if (counts.errors > 0) {
    parts.push(`${counts.errors} error${counts.errors === 1 ? "" : "s"}`);
  }
  if (counts.warnings > 0) {
    parts.push(`${counts.warnings} warning${counts.warnings === 1 ? "" : "s"}`);
  }
  return parts.join(" · ");
}

export function PageHeader({
  url,
  title,
  status,
  console: counts,
  consoleOpen = false,
  onToggleConsole,
}: PageHeaderProps) {
  const statusBadge =
    typeof status === "number" && (status < 200 || status > 299)
      ? { value: status, ...statusDetails(status) }
      : null;
  const showConsole = counts.errors > 0 || counts.warnings > 0;

  return (
    <Flex
      align="center"
      className={styles.header}
      data-testid="browser-page-header"
      gap="2"
      wrap="wrap"
    >
      <span className={styles.icon}>
        <Icon icon={Globe} size="sm" tone="muted" />
      </span>
      {url ? (
        <span className={classNames(styles.url, "rf-truncate")} title={url}>
          {url}
        </span>
      ) : null}
      {title ? (
        <span className={classNames(styles.title, "rf-truncate")}>{title}</span>
      ) : null}
      {statusBadge ? (
        <Badge
          aria-label={`HTTP status ${statusBadge.value}`}
          data-status={statusBadge.state}
          data-testid="browser-page-status"
          size="xs"
          tone={statusBadge.tone}
        >
          HTTP {statusBadge.value}
        </Badge>
      ) : null}
      {showConsole ? (
        <button
          aria-expanded={consoleOpen}
          className={styles.consoleChip}
          data-testid="browser-page-console"
          data-tone={counts.errors > 0 ? "danger" : "warning"}
          onClick={onToggleConsole}
          type="button"
        >
          <Icon
            icon={counts.errors > 0 ? CircleAlert : TriangleAlert}
            size="sm"
          />
          {countsLabel(counts)}
        </button>
      ) : null}
    </Flex>
  );
}
