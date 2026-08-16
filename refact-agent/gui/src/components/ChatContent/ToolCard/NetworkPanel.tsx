import { Box, Flex, Text, TextField } from "@radix-ui/themes";
import React from "react";

import { Badge, Switch } from "../../ui";
import { AnimatedCollapsible } from "../shared/AnimatedCollapsible";
import styles from "./NetworkPanel.module.css";

interface DisplayNetworkEntry {
  method: string;
  url: string;
  resourceType: string;
  status: number | null;
  statusText: string | null;
  redirectFrom: string | null;
  durationMs: number | null;
  size: number | null;
  failureText: string | null;
  fromServiceWorker: boolean;
  isNavigationRequest: boolean;
}

export interface NetworkPanelProps {
  entries?: unknown;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function finiteNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function nonNegativeNumber(value: unknown): number | null {
  const number = finiteNumber(value);
  return number !== null && number >= 0 ? number : null;
}

function optionalText(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function normalizeStatus(value: unknown): number | null {
  const status = finiteNumber(value);
  return status !== null && Number.isInteger(status) && status >= 0
    ? status
    : null;
}

function normalizeDuration(value: unknown): number | null {
  if (!isRecord(value)) return null;
  const start = finiteNumber(value.start_time);
  const end = finiteNumber(value.response_end);
  if (start === null || end === null || end < start) return null;
  return (end - start) * 1_000;
}

function normalizeEntry(value: unknown): DisplayNetworkEntry | null {
  if (!isRecord(value)) return null;
  const status = normalizeStatus(value.status);
  return {
    method: optionalText(value.method) ?? "GET",
    url: optionalText(value.url) ?? "Unknown URL",
    resourceType: optionalText(value.resource_type) ?? "Other",
    status,
    statusText: optionalText(value.status_text),
    redirectFrom: optionalText(value.redirect_from),
    durationMs: normalizeDuration(value.timing),
    size:
      nonNegativeNumber(value.transfer_size) ??
      nonNegativeNumber(value.encoded_data_length),
    failureText: optionalText(value.failure_text),
    fromServiceWorker: value.from_service_worker === true,
    isNavigationRequest: value.is_navigation_request === true,
  };
}

function normalizeEntries(value: unknown): DisplayNetworkEntry[] {
  if (!Array.isArray(value)) return [];
  return value
    .map(normalizeEntry)
    .filter((entry): entry is DisplayNetworkEntry => entry !== null);
}

function isErrorEntry(entry: DisplayNetworkEntry): boolean {
  return (
    entry.failureText !== null || (entry.status !== null && entry.status >= 400)
  );
}

function statusDetails(entry: DisplayNetworkEntry) {
  if (isErrorEntry(entry)) {
    return {
      label: entry.status === null ? "Failed" : String(entry.status),
      tone: "danger" as const,
      state: "error",
    };
  }
  if (entry.status !== null && entry.status >= 300) {
    return {
      label: String(entry.status),
      tone: "accent" as const,
      state: "info",
    };
  }
  if (entry.status !== null && entry.status >= 200) {
    return {
      label: String(entry.status),
      tone: "success" as const,
      state: "success",
    };
  }
  return {
    label: entry.status === null ? "Pending" : String(entry.status),
    tone: "muted" as const,
    state: "neutral",
  };
}

function shortenUrl(value: string): string {
  const maximumLength = 80;
  const endingLength = 24;
  if (value.length <= maximumLength) return value;
  return `${value.slice(0, maximumLength - endingLength - 1)}…${value.slice(
    -endingLength,
  )}`;
}

function formatDuration(value: number | null): string {
  if (value === null) return "—";
  if (value < 1_000) return `${Math.round(value)} ms`;
  return `${(value / 1_000).toFixed(value < 10_000 ? 2 : 1)} s`;
}

function formatSize(value: number | null): string {
  if (value === null) return "—";
  if (value < 1_024) return `${Math.round(value)} B`;
  if (value < 1_048_576) return `${(value / 1_024).toFixed(1)} KB`;
  return `${(value / 1_048_576).toFixed(1)} MB`;
}

export function NetworkPanel({ entries }: NetworkPanelProps) {
  const normalizedEntries = React.useMemo(
    () => normalizeEntries(entries),
    [entries],
  );
  const [filter, setFilter] = React.useState("");
  const [errorsOnly, setErrorsOnly] = React.useState(false);
  const filteredEntries = React.useMemo(() => {
    const query = filter.trim().toLocaleLowerCase();
    return normalizedEntries.filter(
      (entry) =>
        (!query || entry.url.toLocaleLowerCase().includes(query)) &&
        (!errorsOnly || isErrorEntry(entry)),
    );
  }, [errorsOnly, filter, normalizedEntries]);
  const hasErrors = normalizedEntries.some(isErrorEntry);

  if (normalizedEntries.length === 0) return null;

  return (
    <Box className={styles.section}>
      <AnimatedCollapsible
        className={styles.panel}
        data-testid="network-panel"
        header={`Network (${normalizedEntries.length})`}
        status={hasErrors ? "error" : "success"}
        variant="compact"
      >
        <Flex className={styles.filters} align="center" gap="2" wrap="wrap">
          <TextField.Root
            aria-label="Filter network entries by URL"
            className={styles.filterInput}
            placeholder="Filter by URL"
            size="1"
            value={filter}
            onChange={(event) => setFilter(event.currentTarget.value)}
          />
          <Switch
            checked={errorsOnly}
            label="Errors only"
            onCheckedChange={setErrorsOnly}
          />
        </Flex>
        {filteredEntries.length > 0 ? (
          <Box
            aria-label="Network entries"
            className={styles.scrollRegion}
            role="region"
          >
            <ol className={styles.list}>
              {filteredEntries.map((entry, index) => {
                const status = statusDetails(entry);
                const failed = isErrorEntry(entry);
                return (
                  <li
                    className={styles.row}
                    data-error={failed}
                    data-status={status.state}
                    data-testid={`network-entry-${index}`}
                    key={`${entry.url}-${entry.method}-${index}`}
                  >
                    <Badge
                      className={styles.method}
                      size="xs"
                      variant="outline"
                    >
                      {entry.method}
                    </Badge>
                    <span className={styles.url} title={entry.url}>
                      {shortenUrl(entry.url)}
                    </span>
                    <Badge
                      aria-label={`Status ${status.label}`}
                      size="xs"
                      tone={status.tone}
                    >
                      {status.label}
                    </Badge>
                    <Flex className={styles.meta} gap="2" wrap="wrap">
                      <span>{entry.resourceType}</span>
                      <span>{formatDuration(entry.durationMs)}</span>
                      <span>{formatSize(entry.size)}</span>
                      {entry.isNavigationRequest ? (
                        <span>Navigation</span>
                      ) : null}
                      {entry.fromServiceWorker ? (
                        <span>Service worker</span>
                      ) : null}
                      {entry.statusText ? (
                        <span>{entry.statusText}</span>
                      ) : null}
                    </Flex>
                    {entry.redirectFrom ? (
                      <Text
                        className={styles.redirect}
                        size="1"
                        title={entry.redirectFrom}
                      >
                        Redirected from {shortenUrl(entry.redirectFrom)}
                      </Text>
                    ) : null}
                    {entry.failureText ? (
                      <Text className={styles.failure} size="1">
                        {entry.failureText}
                      </Text>
                    ) : null}
                  </li>
                );
              })}
            </ol>
          </Box>
        ) : (
          <Text className={styles.empty} size="1">
            No network entries match these filters.
          </Text>
        )}
      </AnimatedCollapsible>
    </Box>
  );
}
