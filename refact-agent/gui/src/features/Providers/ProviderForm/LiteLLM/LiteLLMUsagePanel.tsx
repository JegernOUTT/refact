import React from "react";

import {
  Badge,
  Button,
  FieldText,
  SegmentedControl,
  Surface,
} from "../../../../components/ui";
import {
  useGetLiteLLMUsageQuery,
  type LiteLLMScope,
  type LiteLLMUsageResponse,
} from "../../../../services/refact/providers";
import {
  FEATURE_STATE_LABEL,
  SCOPE_LABEL,
  featureStateExplanation,
  featureStateTone,
  formatCount,
  formatUsd,
  formatWhen,
  localDateString,
  validateDateRange,
} from "./litellmFormat";
import styles from "./LiteLLM.module.css";

const PAGE_SIZE = 25;

function defaultRange(): { start: string; end: string } {
  const end = new Date();
  const start = new Date();
  start.setDate(end.getDate() - 6);
  return { start: localDateString(start), end: localDateString(end) };
}

function money(value: number | null | undefined): string {
  return typeof value === "number" ? formatUsd(value) : "Not reported";
}

const UsageResult: React.FC<{ usage: LiteLLMUsageResponse }> = ({ usage }) => {
  const totals = usage.totals;
  return (
    <>
      {usage.truncated === true ||
      usage.has_more === true ||
      usage.partial === true ? (
        <div className={styles.notice} role="status">
          {usage.truncated === true || usage.partial === true
            ? "Breakdowns may be incomplete."
            : "More pages are available for this range."}{" "}
          {usage.totals_source === "gateway_summary"
            ? "Totals come from the gateway summary for the whole range."
            : usage.totals_source === "page"
              ? "Totals cover the current page only."
              : "Totals may cover only the data received."}
        </div>
      ) : null}
      {totals ? (
        <dl className={styles.facts} aria-label="Usage totals">
          <div className={styles.fact}>
            <dt>Spend</dt>
            <dd>{money(totals.spend)}</dd>
          </div>
          <div className={styles.fact}>
            <dt>Requests</dt>
            <dd>{formatCount(totals.requests)}</dd>
          </div>
          <div className={styles.fact}>
            <dt>Prompt tokens</dt>
            <dd>{formatCount(totals.prompt_tokens)}</dd>
          </div>
          <div className={styles.fact}>
            <dt>Completion tokens</dt>
            <dd>{formatCount(totals.completion_tokens)}</dd>
          </div>
          <div className={styles.fact}>
            <dt>Total tokens</dt>
            <dd>{formatCount(totals.total_tokens)}</dd>
          </div>
          {totals.failed_requests !== undefined ? (
            <div className={styles.fact}>
              <dt>Failed requests</dt>
              <dd>{formatCount(totals.failed_requests)}</dd>
            </div>
          ) : null}
        </dl>
      ) : null}

      {usage.daily && usage.daily.length > 0 ? (
        <div className={styles.tableWrap}>
          <table className={styles.table}>
            <caption className={styles.meta}>Daily activity</caption>
            <thead>
              <tr>
                <th scope="col">Date</th>
                <th scope="col" className={styles.numeric}>
                  Spend
                </th>
                <th scope="col" className={styles.numeric}>
                  Requests
                </th>
                <th scope="col" className={styles.numeric}>
                  Tokens
                </th>
              </tr>
            </thead>
            <tbody>
              {usage.daily.map((day) => (
                <tr key={day.date}>
                  <th scope="row">{day.date}</th>
                  <td className={styles.numeric}>{money(day.spend)}</td>
                  <td className={styles.numeric}>
                    {formatCount(day.requests)}
                  </td>
                  <td className={styles.numeric}>
                    {formatCount(day.total_tokens)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}

      {usage.models && usage.models.length > 0 ? (
        <div className={styles.tableWrap}>
          <table className={styles.table}>
            <caption className={styles.meta}>By model (this page)</caption>
            <thead>
              <tr>
                <th scope="col">Model</th>
                <th scope="col" className={styles.numeric}>
                  Spend
                </th>
                <th scope="col" className={styles.numeric}>
                  Requests
                </th>
                <th scope="col" className={styles.numeric}>
                  Tokens
                </th>
              </tr>
            </thead>
            <tbody>
              {usage.models.map((model) => (
                <tr key={model.model}>
                  <th scope="row">{model.model}</th>
                  <td className={styles.numeric}>{money(model.spend)}</td>
                  <td className={styles.numeric}>
                    {formatCount(model.requests)}
                  </td>
                  <td className={styles.numeric}>
                    {formatCount(model.total_tokens)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}

      {usage.api_keys && usage.api_keys.length > 0 ? (
        <div className={styles.tableWrap}>
          <table className={styles.table}>
            <caption className={styles.meta}>By API key (this page)</caption>
            <thead>
              <tr>
                <th scope="col">Key</th>
                <th scope="col" className={styles.numeric}>
                  Spend
                </th>
                <th scope="col" className={styles.numeric}>
                  Requests
                </th>
                <th scope="col" className={styles.numeric}>
                  Tokens
                </th>
              </tr>
            </thead>
            <tbody>
              {usage.api_keys.map((key) => (
                <tr key={`${key.label}:${key.alias ?? ""}`}>
                  <th scope="row">
                    {key.alias ?? key.label}
                    {key.current ? " (current credential)" : ""}
                  </th>
                  <td className={styles.numeric}>{money(key.spend)}</td>
                  <td className={styles.numeric}>
                    {formatCount(key.requests)}
                  </td>
                  <td className={styles.numeric}>
                    {formatCount(key.total_tokens)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}

      {totals &&
      (usage.daily?.length ?? 0) === 0 &&
      (usage.models?.length ?? 0) === 0 ? (
        <div className={styles.notice}>
          The gateway reported no activity in this range.
        </div>
      ) : null}
    </>
  );
};

export const LiteLLMUsagePanel: React.FC<{ providerName: string }> = ({
  providerName,
}) => {
  const [range, setRange] = React.useState(defaultRange);
  const [scope, setScope] = React.useState<LiteLLMScope>("key");
  const [page, setPage] = React.useState(1);

  const rangeError = validateDateRange(range.start, range.end);
  const { currentData, isError, isFetching, isLoading, refetch } =
    useGetLiteLLMUsageQuery(
      {
        providerName,
        start: range.start,
        end: range.end,
        scope,
        page,
        pageSize: PAGE_SIZE,
      },
      { skip: rangeError !== null },
    );
  const fetchedAt = formatWhen(currentData?.fetched_at);
  const state = currentData?.state;
  const totalPages = currentData?.total_pages ?? null;
  const hasNext =
    currentData?.has_more === true ||
    (totalPages !== null && page < totalPages);

  return (
    <Surface className={styles.section} variant="glass">
      <div className={styles.header}>
        <div>
          <div className={styles.title}>Usage</div>
          <div className={styles.meta}>
            Aggregate activity for the {SCOPE_LABEL[scope]} scope, read with the
            configured provider credential or login. Scopes are independent and
            never combined. No prompts or completions are fetched.
            {fetchedAt ? ` Updated ${fetchedAt}.` : ""}
          </div>
        </div>
        <div className={styles.badges}>
          {isFetching && !isLoading ? (
            <Badge tone="accent">Refreshing</Badge>
          ) : null}
          {state && state !== "ok" ? (
            <Badge tone={featureStateTone(state)}>
              {FEATURE_STATE_LABEL[state]}
            </Badge>
          ) : null}
          <Button
            size="1"
            variant="ghost"
            disabled={isFetching || rangeError !== null}
            onClick={() => void refetch()}
          >
            Refresh
          </Button>
        </div>
      </div>

      <div className={styles.filters}>
        <label className={styles.filterField}>
          Start date
          <FieldText
            type="date"
            value={range.start}
            max={range.end}
            aria-label="Start date"
            onChange={(start) => {
              setRange((prev) => ({ ...prev, start }));
              setPage(1);
            }}
          />
        </label>
        <label className={styles.filterField}>
          End date
          <FieldText
            type="date"
            value={range.end}
            min={range.start}
            aria-label="End date"
            onChange={(end) => {
              setRange((prev) => ({ ...prev, end }));
              setPage(1);
            }}
          />
        </label>
        <SegmentedControl
          aria-label="Usage scope"
          size="sm"
          layout="content"
          value={scope}
          options={(["key", "user", "team"] as const).map((value) => ({
            value,
            label: SCOPE_LABEL[value],
          }))}
          onValueChange={(value) => {
            setScope(value as LiteLLMScope);
            setPage(1);
          }}
        />
      </div>

      {rangeError ? (
        <div className={styles.noticeDanger} role="alert">
          {rangeError}
        </div>
      ) : null}
      {isLoading ? <div className={styles.meta}>Loading usage…</div> : null}
      {isError ? (
        <div className={styles.noticeDanger} role="alert">
          Failed to load usage.
        </div>
      ) : null}
      {currentData && currentData.state !== "ok" ? (
        <div className={styles.notice} role="status">
          {currentData.message ??
            featureStateExplanation(currentData.state, "usage history")}
        </div>
      ) : null}

      {currentData?.state === "ok" ? <UsageResult usage={currentData} /> : null}

      {currentData?.state === "ok" && (page > 1 || hasNext) ? (
        <div className={styles.actions}>
          <Button
            size="1"
            variant="soft"
            disabled={page <= 1 || isFetching}
            onClick={() => setPage((value) => Math.max(1, value - 1))}
          >
            Previous page
          </Button>
          <span className={styles.meta}>
            Page {page}
            {totalPages ? ` of ${totalPages}` : ""}
          </span>
          <Button
            size="1"
            variant="soft"
            disabled={!hasNext || isFetching}
            onClick={() => setPage((value) => value + 1)}
          >
            Next page
          </Button>
        </div>
      ) : null}
    </Surface>
  );
};
