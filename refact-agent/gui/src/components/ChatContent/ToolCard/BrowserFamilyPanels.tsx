import React from "react";
import { Box, Flex, TextField } from "@radix-ui/themes";
import { ArrowDown, ArrowUp, Clock, Radio } from "lucide-react";

import { Badge, Chip, Icon, StatusDot } from "../../ui";
import type { BadgeTone } from "../../ui";
import type { StatusDotStatus } from "../../ui/StatusDot/statusTone";
import { ShikiCodeBlock } from "../../Markdown";
import { AnimatedCollapsible } from "../shared/AnimatedCollapsible";
import type {
  CdpEntry,
  ClockEntry,
  DeviceCatalog,
  EmulationChip,
  HttpRequestEntry,
  NetworkSummaryRow,
  ReadoutEntry,
  ResetEntry,
  RouteChainEntry,
  WebSocketCloseRow,
  WebSocketEventRow,
  WebSocketRouteRow,
} from "./browserFamilies";
import styles from "./BrowserFamilyPanels.module.css";

function shortenUrl(value: string, maximumLength = 72): string {
  if (value.length <= maximumLength) return value;
  const endingLength = 20;
  return `${value.slice(0, maximumLength - endingLength - 1)}…${value.slice(
    -endingLength,
  )}`;
}

function statusTone(status: number | null): BadgeTone {
  if (status === null) return "muted";
  if (status >= 400) return "danger";
  if (status >= 300) return "accent";
  if (status >= 200) return "success";
  return "muted";
}

function formatBytes(value: number | null): string {
  if (value === null) return "—";
  if (value < 1_024) return `${Math.round(value)} B`;
  if (value < 1_048_576) return `${(value / 1_024).toFixed(1)} KB`;
  return `${(value / 1_048_576).toFixed(1)} MB`;
}

function formatMs(value: number | null): string {
  if (value === null) return "—";
  if (value < 1_000) return `${Math.round(value)} ms`;
  return `${(value / 1_000).toFixed(2)} s`;
}

interface SectionProps {
  label: string;
  testId: string;
  children: React.ReactNode;
}

function Section({ label, testId, children }: SectionProps) {
  return (
    <Box className={styles.section} data-testid={testId}>
      <Box className={styles.sectionLabel}>{label}</Box>
      {children}
    </Box>
  );
}

export interface NetworkSummaryPanelProps {
  rows: NetworkSummaryRow[];
}

export function NetworkSummaryPanel({ rows }: NetworkSummaryPanelProps) {
  if (rows.length === 0) return null;

  return (
    <Section
      label={`Network summary (${rows.length})`}
      testId="network-summary"
    >
      <Box className={styles.rowList}>
        {rows.map((row, index) => (
          <Box
            className={styles.networkRow}
            data-error={row.failure !== null}
            data-testid={`network-summary-row-${index}`}
            key={`${row.raw}-${index}`}
          >
            {row.method ? (
              <Badge className={styles.mono} size="xs" variant="outline">
                {row.method}
              </Badge>
            ) : null}
            <span className={styles.url} title={row.url}>
              {shortenUrl(row.url)}
            </span>
            <Badge size="xs" tone={statusTone(row.status)}>
              {row.status ?? "—"}
            </Badge>
            <Flex className={styles.rowMeta} gap="2" wrap="wrap">
              <span>{formatBytes(row.bytes)}</span>
              <span>{formatMs(row.elapsedMs)}</span>
            </Flex>
            {row.failure ? (
              <Box className={styles.failure}>{row.failure}</Box>
            ) : null}
          </Box>
        ))}
      </Box>
    </Section>
  );
}

export interface RouteChainPanelProps {
  chains: RouteChainEntry[];
}

export function RouteChainPanel({ chains }: RouteChainPanelProps) {
  if (chains.length === 0) return null;

  return (
    <Section label="Route chain" testId="route-chain">
      <Box className={styles.rowList}>
        {chains.map((chain) =>
          chain.routes.length === 0 ? (
            <Box className={styles.empty} key={chain.stepIndex}>
              No active routes
            </Box>
          ) : (
            <Box className={styles.chain} key={chain.stepIndex}>
              {chain.routes.map((route) => (
                <Flex
                  align="baseline"
                  className={styles.routeRow}
                  data-testid="route-chain-row"
                  gap="2"
                  key={`${chain.stepIndex}-${route.order}`}
                  wrap="wrap"
                >
                  <Badge size="xs" tone="muted">
                    {route.order + 1}
                  </Badge>
                  <span className={styles.mono}>{route.pattern}</span>
                  <Badge size="xs" tone="accent">
                    {route.handler}
                  </Badge>
                  {route.detail ? (
                    <span className={styles.rowMeta}>{route.detail}</span>
                  ) : null}
                  {route.timesRemaining !== null ? (
                    <Badge size="xs" tone="warning">
                      {route.timesRemaining} left
                    </Badge>
                  ) : null}
                  {route.harEntries !== null ? (
                    <Badge size="xs" tone="success">
                      HAR · {route.harEntries} entries
                    </Badge>
                  ) : null}
                </Flex>
              ))}
            </Box>
          ),
        )}
      </Box>
    </Section>
  );
}

export interface HttpRequestPanelProps {
  entries: HttpRequestEntry[];
}

export function HttpRequestPanel({ entries }: HttpRequestPanelProps) {
  if (entries.length === 0) return null;

  return (
    <Section label="HTTP requests" testId="http-requests">
      <Box className={styles.rowList}>
        {entries.map((entry) => (
          <Box
            className={styles.httpRow}
            data-testid="http-request-row"
            key={entry.stepIndex}
          >
            <Flex align="baseline" gap="2" wrap="wrap">
              <Badge className={styles.mono} size="xs" variant="outline">
                {entry.method}
              </Badge>
              <span className={styles.url} title={entry.url}>
                {shortenUrl(entry.url)}
              </span>
              <Badge size="xs" tone={statusTone(entry.status)}>
                {entry.status}
                {entry.statusText ? ` ${entry.statusText}` : ""}
              </Badge>
            </Flex>
            <Flex className={styles.rowMeta} gap="2" wrap="wrap">
              <span>{formatBytes(entry.bodyBytes)}</span>
              {entry.redirects !== null && entry.redirects > 0 ? (
                <span>{entry.redirects} redirects</span>
              ) : null}
              {entry.cookieNames.length > 0 ? (
                <span>Set-Cookie: {entry.cookieNames.join(", ")}</span>
              ) : null}
            </Flex>
            {entry.artifactPath ? (
              <Chip className={styles.mono} radius="chip">
                {entry.artifactPath}
                {entry.artifactBytes === null
                  ? ""
                  : ` · ${formatBytes(entry.artifactBytes)}`}
              </Chip>
            ) : null}
            {entry.body ? (
              <AnimatedCollapsible
                data-testid="http-request-body"
                header="Response body"
                variant="compact"
              >
                <Box className={styles.codeBlock}>
                  <ShikiCodeBlock
                    className={`language-${entry.bodyLanguage}`}
                    showLineNumbers={false}
                  >
                    {entry.body}
                  </ShikiCodeBlock>
                </Box>
              </AnimatedCollapsible>
            ) : null}
          </Box>
        ))}
      </Box>
    </Section>
  );
}

export interface ClockTimelineProps {
  entries: ClockEntry[];
}

export function ClockTimeline({ entries }: ClockTimelineProps) {
  if (entries.length === 0) return null;

  return (
    <Section label="Clock" testId="clock-timeline">
      <Flex gap="2" wrap="wrap">
        {entries.map((entry) => (
          <Chip
            data-testid="clock-chip"
            icon={<Icon icon={Clock} size="sm" />}
            key={entry.stepIndex}
          >
            {entry.action}
            {entry.offset ? ` ${entry.offset}` : ""}
            {entry.paused ? " · paused" : ""}
          </Chip>
        ))}
      </Flex>
    </Section>
  );
}

const STATE_LABELS: [keyof ReadoutStateFlags, string][] = [
  ["visible", "visible"],
  ["enabled", "enabled"],
  ["editable", "editable"],
  ["stable", "stable"],
];

type ReadoutStateFlags = {
  visible?: boolean;
  enabled?: boolean;
  editable?: boolean;
  stable?: boolean;
};

function flagStatus(value: boolean | undefined): StatusDotStatus {
  if (value === undefined) return "idle";
  return value ? "success" : "error";
}

export interface ReadoutPanelProps {
  entries: ReadoutEntry[];
}

export function ReadoutPanel({ entries }: ReadoutPanelProps) {
  if (entries.length === 0) return null;

  return (
    <Section label="Readouts" testId="readouts">
      <Box className={styles.rowList}>
        {entries.map((entry) => (
          <Box
            className={styles.readoutRow}
            data-testid={`readout-${entry.kind}`}
            key={entry.stepIndex}
          >
            <Box className={styles.readoutSummary}>{entry.summary}</Box>
            {entry.kind === "bounding_box" ? (
              <Flex gap="1" wrap="wrap">
                {entry.box === null ? (
                  <Badge size="xs" tone="warning">
                    no bounding box
                  </Badge>
                ) : (
                  <>
                    <Chip radius="chip">x {entry.box.x}</Chip>
                    <Chip radius="chip">y {entry.box.y}</Chip>
                    <Chip radius="chip">w {entry.box.width}</Chip>
                    <Chip radius="chip">h {entry.box.height}</Chip>
                  </>
                )}
              </Flex>
            ) : null}
            {entry.kind === "count" ? (
              <Badge size="xs" tone="accent">
                {entry.count} matched
              </Badge>
            ) : null}
            {entry.kind === "value" || entry.kind === "attribute" ? (
              <Flex align="baseline" gap="2" wrap="wrap">
                {entry.label ? (
                  <Badge className={styles.mono} size="xs" variant="outline">
                    {entry.label}
                  </Badge>
                ) : null}
                <span className={styles.mono}>{entry.value}</span>
              </Flex>
            ) : null}
            {entry.kind === "state" && entry.state ? (
              <Flex gap="3" wrap="wrap">
                {STATE_LABELS.map(([key, label]) => (
                  <Flex align="center" gap="1" key={label}>
                    <StatusDot
                      data-testid={`readout-state-${label}`}
                      status={flagStatus(entry.state?.[key])}
                    />
                    <span className={styles.rowMeta}>{label}</span>
                  </Flex>
                ))}
                {entry.state.checked === null ||
                entry.state.checked === undefined ? null : (
                  <Badge size="xs" tone="muted">
                    checked: {String(entry.state.checked)}
                  </Badge>
                )}
              </Flex>
            ) : null}
          </Box>
        ))}
      </Box>
    </Section>
  );
}

export interface DevicePanelProps {
  catalogs: DeviceCatalog[];
  emulation: EmulationChip[];
}

export function DevicePanel({ catalogs, emulation }: DevicePanelProps) {
  const [filter, setFilter] = React.useState("");
  const query = filter.trim().toLocaleLowerCase();
  const devices = React.useMemo(() => {
    const names = catalogs.flatMap((catalog) => catalog.devices);
    return names.filter((name) => name.toLocaleLowerCase().includes(query));
  }, [catalogs, query]);
  const aliases = catalogs.flatMap((catalog) => catalog.aliases);

  if (catalogs.length === 0 && emulation.length === 0) return null;

  return (
    <Section label="Devices" testId="devices">
      {emulation.length > 0 ? (
        <Flex gap="2" wrap="wrap">
          {emulation.map((chip) => (
            <Chip data-testid="emulation-chip" key={chip.stepIndex}>
              {chip.label}
            </Chip>
          ))}
        </Flex>
      ) : null}
      {catalogs.length > 0 ? (
        <>
          <TextField.Root
            aria-label="Filter devices"
            className={styles.filterInput}
            placeholder="Filter devices"
            size="1"
            value={filter}
            onChange={(event) => setFilter(event.currentTarget.value)}
          />
          {aliases.length > 0 ? (
            <Flex gap="2" wrap="wrap">
              {aliases.map((alias) => (
                <Badge key={alias} size="xs" tone="muted">
                  {alias}
                </Badge>
              ))}
            </Flex>
          ) : null}
          {devices.length > 0 ? (
            <Flex className={styles.deviceCloud} gap="1" wrap="wrap">
              {devices.map((name) => (
                <Chip data-testid="device-chip" key={name}>
                  {name}
                </Chip>
              ))}
            </Flex>
          ) : (
            <Box className={styles.empty}>No devices match this filter.</Box>
          )}
        </>
      ) : null}
    </Section>
  );
}

const WEB_SOCKET_DIRECTION_LABEL: Record<
  WebSocketEventRow["direction"],
  string
> = {
  sent: "sent",
  received: "received",
  none: "event",
};

export interface WebSocketPanelProps {
  routes: WebSocketRouteRow[];
  events: WebSocketEventRow[];
  closes: WebSocketCloseRow[];
}

export function WebSocketPanel({
  routes,
  events,
  closes,
}: WebSocketPanelProps) {
  if (routes.length === 0 && events.length === 0 && closes.length === 0) {
    return null;
  }

  return (
    <Section label="WebSockets" testId="websockets">
      {routes.length > 0 ? (
        <Flex gap="2" wrap="wrap">
          {routes.map((route) => (
            <Flex
              align="baseline"
              data-testid="websocket-route"
              gap="2"
              key={route.stepIndex}
              wrap="wrap"
            >
              <Badge
                size="xs"
                tone={route.mode === "intercept" ? "accent" : "warning"}
              >
                {route.mode}
              </Badge>
              <span className={styles.mono}>{route.pattern}</span>
            </Flex>
          ))}
        </Flex>
      ) : null}
      {events.length > 0 ? (
        <Box className={styles.rowList}>
          {events.map((event) => (
            <Flex
              align="baseline"
              className={styles.websocketRow}
              data-status={event.failed ? "error" : "idle"}
              data-testid="websocket-event"
              gap="2"
              key={event.sequence}
              wrap="wrap"
            >
              <Icon
                icon={
                  event.direction === "sent"
                    ? ArrowUp
                    : event.direction === "received"
                      ? ArrowDown
                      : Radio
                }
                size="sm"
                tone={event.failed ? "danger" : "muted"}
              />
              <Badge size="xs" tone={event.failed ? "danger" : "muted"}>
                {WEB_SOCKET_DIRECTION_LABEL[event.direction]}
              </Badge>
              <span className={styles.url} title={event.url}>
                {shortenUrl(event.url)}
              </span>
              {event.routed ? (
                <Badge size="xs" tone="accent">
                  routed
                </Badge>
              ) : null}
              {event.detail ? (
                <span className={styles.rowMeta}>{event.detail}</span>
              ) : null}
            </Flex>
          ))}
        </Box>
      ) : null}
      {closes.map((close) => (
        <Flex align="baseline" gap="2" key={close.stepIndex} wrap="wrap">
          <Badge size="xs" tone="muted">
            closed {close.closed}
          </Badge>
          {close.code !== null ? (
            <Badge size="xs" tone="warning">
              code {close.code}
            </Badge>
          ) : null}
          {close.reason ? (
            <span className={styles.rowMeta}>{close.reason}</span>
          ) : null}
        </Flex>
      ))}
    </Section>
  );
}

export interface CdpPanelProps {
  entries: CdpEntry[];
}

export function CdpPanel({ entries }: CdpPanelProps) {
  if (entries.length === 0) return null;

  return (
    <Section label="CDP" testId="cdp">
      <Box className={styles.rowList}>
        {entries.map((entry) => (
          <Box
            className={styles.cdpRow}
            data-testid="cdp-row"
            key={entry.stepIndex}
          >
            <Flex align="baseline" gap="2" wrap="wrap">
              <Badge className={styles.mono} size="xs" variant="outline">
                {entry.method}
              </Badge>
              <Badge size="xs" tone="muted">
                {entry.target}
              </Badge>
              <span className={styles.rowMeta}>{formatBytes(entry.bytes)}</span>
            </Flex>
            {entry.warnings.map((warning, index) => (
              <Box className={styles.warning} key={index}>
                {warning}
              </Box>
            ))}
            {entry.artifactPath ? (
              <Chip className={styles.mono} radius="chip">
                {entry.artifactPath}
              </Chip>
            ) : null}
            {entry.result ? (
              <AnimatedCollapsible
                data-testid="cdp-result"
                header="Result"
                variant="compact"
              >
                <Box className={styles.codeBlock}>
                  <ShikiCodeBlock
                    className="language-json"
                    showLineNumbers={false}
                  >
                    {entry.result}
                  </ShikiCodeBlock>
                </Box>
              </AnimatedCollapsible>
            ) : null}
          </Box>
        ))}
      </Box>
    </Section>
  );
}

export interface ResetPanelProps {
  entries: ResetEntry[];
}

export function ResetPanel({ entries }: ResetPanelProps) {
  if (entries.length === 0) return null;

  return (
    <Section label="Reset" testId="reset">
      {entries.map((entry) => (
        <Flex gap="2" key={entry.stepIndex} wrap="wrap">
          {entry.cleared.map((item) => (
            <Chip
              data-testid="reset-chip"
              key={item.label}
              selected={item.count > 0}
            >
              {item.count} {item.label}
            </Chip>
          ))}
          {entry.flags.map((flag) => (
            <Badge key={flag} size="xs" tone="success">
              {flag}
            </Badge>
          ))}
        </Flex>
      ))}
    </Section>
  );
}
