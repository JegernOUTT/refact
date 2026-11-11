import React from "react";

import {
  Badge,
  Button,
  ProviderQuota,
  Surface,
} from "../../../../components/ui";
import {
  useGetLiteLLMAccountQuery,
  type LiteLLMAccountSubject,
  type LiteLLMObservedHeadroom,
} from "../../../../services/refact/providers";
import {
  FEATURE_STATE_LABEL,
  SCOPE_LABEL,
  featureStateExplanation,
  featureStateTone,
  formatCount,
  formatLimit,
  formatUsd,
  formatWhen,
} from "./litellmFormat";
import styles from "./LiteLLM.module.css";

const ACCOUNT_POLL_MS = 60_000;

function Fact({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className={styles.fact}>
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}

function spendText(subject: LiteLLMAccountSubject): string {
  const spend = subject.spend;
  if (!spend || spend.state === "unknown" || spend.state === "absent") {
    return "Not reported";
  }
  if (spend.value === null || spend.value === undefined) return "Not reported";
  return formatUsd(spend.value, subject.currency);
}

function budgetText(subject: LiteLLMAccountSubject): string {
  const budget = subject.max_budget;
  if (!budget) return "Not reported";
  switch (budget.kind) {
    case "unlimited":
      return "Unlimited";
    case "denied":
      return "Permission denied";
    case "limited":
      return typeof budget.value === "number"
        ? formatUsd(budget.value, subject.currency)
        : "Not reported";
    default:
      return "Unknown";
  }
}

function remainingText(subject: LiteLLMAccountSubject): string {
  const remaining = subject.remaining;
  if (!remaining || typeof remaining.value !== "number") {
    return subject.max_budget?.kind === "unlimited"
      ? "Not applicable"
      : "Not reported";
  }
  const value = formatUsd(remaining.value, subject.currency);
  const notes = [
    remaining.derived ? "derived" : null,
    remaining.overage === true || remaining.value < 0 ? "over budget" : null,
  ].filter(Boolean);
  return notes.length > 0 ? `${value} (${notes.join(", ")})` : value;
}

function usedPercent(subject: LiteLLMAccountSubject): number | undefined {
  const budget = subject.max_budget;
  const spend = subject.spend?.value;
  if (
    budget?.kind !== "limited" ||
    typeof budget.value !== "number" ||
    budget.value <= 0 ||
    typeof spend !== "number"
  ) {
    return undefined;
  }
  return Math.max(0, (spend / budget.value) * 100);
}

const SubjectCard: React.FC<{ subject: LiteLLMAccountSubject }> = ({
  subject,
}) => {
  const percent = usedPercent(subject);
  const tone =
    percent === undefined
      ? "muted"
      : percent >= 100
        ? "danger"
        : percent >= 80
          ? "warning"
          : "accent";
  const explanation =
    subject.state === "ok"
      ? null
      : subject.message ?? featureStateExplanation(subject.state, "this scope");
  const showFigures = subject.state === "ok";
  const resetAt = formatWhen(subject.budget_reset_at);
  const expiresAt = formatWhen(subject.expires_at);
  return (
    <section
      className={styles.card}
      aria-label={`${SCOPE_LABEL[subject.scope]} budget`}
      data-testid={`litellm-subject-${subject.scope}`}
    >
      <div className={styles.header}>
        <div className={styles.title}>
          {SCOPE_LABEL[subject.scope]} · {subject.label}
        </div>
        <div className={styles.badges}>
          {subject.state !== "ok" ? (
            <Badge tone={featureStateTone(subject.state)}>
              {FEATURE_STATE_LABEL[subject.state]}
            </Badge>
          ) : null}
          {subject.blocked ? <Badge tone="danger">Blocked</Badge> : null}
        </div>
      </div>
      {explanation ? <div className={styles.notice}>{explanation}</div> : null}
      {showFigures ? (
        <>
          <ProviderQuota
            label="Spend"
            value={`${spendText(subject)} of ${budgetText(subject)}`}
            usedPercent={percent}
            tone={tone}
            meta={
              percent !== undefined && percent > 100
                ? `${Math.round(percent)}% of budget`
                : undefined
            }
          />
          <dl className={styles.facts}>
            <Fact label="Spend" value={spendText(subject)} />
            <Fact label="Max budget" value={budgetText(subject)} />
            <Fact label="Remaining" value={remainingText(subject)} />
            <Fact label="Budget resets" value={resetAt ?? "Not reported"} />
            <Fact
              label="Budget duration"
              value={subject.budget_duration ?? "Not reported"}
            />
            {subject.scope === "key" ? (
              <Fact label="Key expires" value={expiresAt ?? "Not reported"} />
            ) : null}
            <Fact
              label="Configured RPM limit"
              value={formatLimit(subject, "rpm_limit", subject.rpm_limit)}
            />
            <Fact
              label="Configured TPM limit"
              value={formatLimit(subject, "tpm_limit", subject.tpm_limit)}
            />
            <Fact
              label="Max parallel requests"
              value={formatLimit(
                subject,
                "max_parallel_requests",
                subject.max_parallel_requests,
              )}
            />
          </dl>
          {subject.model_limits && subject.model_limits.length > 0 ? (
            <div className={styles.tableWrap}>
              <table className={styles.table}>
                <caption className={styles.meta}>Per-model limits</caption>
                <thead>
                  <tr>
                    <th scope="col">Model</th>
                    <th scope="col" className={styles.numeric}>
                      Budget
                    </th>
                    <th scope="col" className={styles.numeric}>
                      RPM
                    </th>
                    <th scope="col" className={styles.numeric}>
                      TPM
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {subject.model_limits.map((limit) => (
                    <tr key={limit.model}>
                      <th scope="row">{limit.model}</th>
                      <td className={styles.numeric}>
                        {typeof limit.max_budget === "number"
                          ? formatUsd(limit.max_budget, subject.currency)
                          : "Not reported"}
                      </td>
                      <td className={styles.numeric}>
                        {formatLimit(limit, "rpm_limit", limit.rpm_limit)}
                      </td>
                      <td className={styles.numeric}>
                        {formatLimit(limit, "tpm_limit", limit.tpm_limit)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : null}
        </>
      ) : null}
    </section>
  );
};

const HeadroomCard: React.FC<{ headroom: LiteLLMObservedHeadroom }> = ({
  headroom,
}) => {
  const observed = formatWhen(headroom.observed_at);
  const expires = formatWhen(headroom.expires_at);
  const requestsReset = formatWhen(headroom.requests_reset_at);
  const tokensReset = formatWhen(headroom.tokens_reset_at);
  return (
    <section
      className={styles.card}
      aria-label="Observed rate headroom"
      data-testid="litellm-observed-headroom"
    >
      <div className={styles.header}>
        <div className={styles.title}>Observed rate headroom</div>
        <Badge tone="muted">From last response</Badge>
      </div>
      <div className={styles.notice}>
        Reported by the gateway on a real request
        {headroom.model ? ` (${headroom.model})` : ""}
        {observed ? ` at ${observed}` : ""}. Source scope:{" "}
        {headroom.scope ?? "unknown"} — it may describe an upstream provider and
        is not an account balance for the key, user or team.
      </div>
      <dl className={styles.facts}>
        <Fact
          label="Requests remaining"
          value={formatCount(headroom.requests_remaining)}
        />
        <Fact
          label="Requests limit"
          value={formatCount(headroom.requests_limit)}
        />
        <Fact label="Requests reset" value={requestsReset ?? "Not reported"} />
        <Fact
          label="Tokens remaining"
          value={formatCount(headroom.tokens_remaining)}
        />
        <Fact label="Tokens limit" value={formatCount(headroom.tokens_limit)} />
        <Fact label="Tokens reset" value={tokensReset ?? "Not reported"} />
        {typeof headroom.response_cost === "number" ? (
          <Fact
            label="Last response cost"
            value={formatUsd(headroom.response_cost)}
          />
        ) : null}
        {expires ? <Fact label="Observation expires" value={expires} /> : null}
      </dl>
    </section>
  );
};

export const LiteLLMAccountPanel: React.FC<{ providerName: string }> = ({
  providerName,
}) => {
  const { currentData, isError, isFetching, isLoading, refetch } =
    useGetLiteLLMAccountQuery(
      { providerName },
      { pollingInterval: ACCOUNT_POLL_MS, skipPollingIfUnfocused: true },
    );
  const fetchedAt = formatWhen(currentData?.fetched_at);

  return (
    <Surface className={styles.section} variant="glass">
      <div className={styles.header}>
        <div>
          <div className={styles.title}>Quotas</div>
          <div className={styles.meta}>
            Budgets are read with the configured provider credential or login.
            Key, user and team limits are independent, may be unavailable
            separately (a login may have no virtual key), and are never summed.
            {fetchedAt ? ` Updated ${fetchedAt}.` : ""}
          </div>
        </div>
        <div className={styles.badges}>
          {isFetching && !isLoading ? (
            <Badge tone="accent">Refreshing</Badge>
          ) : null}
          {currentData?.stale ? <Badge tone="warning">Stale</Badge> : null}
          {currentData && currentData.state !== "ok" ? (
            <Badge tone={featureStateTone(currentData.state)}>
              {FEATURE_STATE_LABEL[currentData.state]}
            </Badge>
          ) : null}
          <Button
            size="1"
            variant="ghost"
            disabled={isFetching}
            onClick={() => void refetch()}
          >
            Refresh
          </Button>
        </div>
      </div>

      {isLoading ? <div className={styles.meta}>Loading quotas…</div> : null}
      {isError ? (
        <div className={styles.noticeDanger} role="alert">
          {currentData
            ? "Failed to refresh quotas; showing the last known values."
            : "Failed to load quotas."}
        </div>
      ) : null}
      {currentData && currentData.state !== "ok" ? (
        <div className={styles.notice} role="status">
          {currentData.message ??
            featureStateExplanation(currentData.state, "account quotas")}
        </div>
      ) : null}

      {currentData?.subjects.map((subject) => (
        <SubjectCard
          key={`${subject.scope}:${subject.id ?? subject.label}`}
          subject={subject}
        />
      ))}
      {currentData?.state === "ok" && currentData.subjects.length === 0 ? (
        <div className={styles.notice}>
          The gateway reported no key, user or team budget for the configured
          credential.
        </div>
      ) : null}
      {currentData?.observed_headroom ? (
        <HeadroomCard headroom={currentData.observed_headroom} />
      ) : null}
    </Surface>
  );
};
