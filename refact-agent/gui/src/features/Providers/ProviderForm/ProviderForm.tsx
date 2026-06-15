import React from "react";
import { v4 as uuidv4 } from "uuid";

import { SchemaField } from "./SchemaField";
import { ProviderOAuth } from "./ProviderOAuth";
import { RoleSeparatedModelConfig } from "./RoleSeparatedModelConfig";
import { Spinner } from "../../../components/Spinner";

import { useProviderForm } from "./useProviderForm";
import type {
  ClaudeCodeUsageData,
  ProviderListItem,
  ProviderStatus,
  ClaudeCodeUsageWindow,
  OpenAICodexAdditionalRateLimit,
  OpenAICodexRateLimit,
  OpenAICodexUsageData,
  OpenAICodexUsageWindow,
} from "../../../services/refact";
import { Badge, Button, Surface } from "../../../components/ui";

import styles from "./ProviderForm.module.css";
import { ProviderModelsList } from "./ProviderModelsList/ProviderModelsList";
import {
  useGetOpenRouterHealthQuery,
  useGetClaudeCodeUsageQuery,
  useGetOpenAICodexUsageQuery,
  useRedeemOpenAICodexResetCreditMutation,
} from "../../../services/refact";
import {
  clampPercent,
  formatClaudeExtraUsage,
  formatCodexCreditsDetails,
  formatCodexCreditsSummary,
  formatCodexSpendControl,
  formatLimitWindowSeconds,
  formatNullableBool,
  formatQuotaMeta,
  formatResetAfterSeconds,
  formatResetAt,
  formatUsagePercent,
  formatWindowLabel,
} from "../../../utils/providerQuota";

export type ProviderFormProps = {
  currentProvider: ProviderListItem;
};

export type { ProviderListItem };

const StatusBadge: React.FC<{ status: ProviderStatus }> = ({ status }) => {
  switch (status) {
    case "active":
      return <Badge tone="success">Active</Badge>;
    case "configured":
      return <Badge tone="warning">Configured</Badge>;
    case "not_configured":
      return <Badge tone="muted">Not configured</Badge>;
    default:
      return null;
  }
};

const UsageBar: React.FC<{ pct: number }> = ({ pct }) => (
  <progress
    className={styles.usageBar}
    max={100}
    value={pct}
    aria-label={`${Math.round(pct)}% used`}
  />
);

const ClaudeWindowRow: React.FC<{
  label: string;
  w: ClaudeCodeUsageWindow;
}> = ({ label, w }) => {
  const pct = clampPercent(w.percent_used);
  const meta = formatQuotaMeta([
    formatUsagePercent(pct),
    formatResetAt(w.resets_at),
  ]);
  return (
    <div className={styles.usageRow}>
      <div className={styles.usageRowHeader}>
        <span>{label}</span>
        <span>{meta}</span>
      </div>
      <UsageBar pct={pct} />
    </div>
  );
};

const CodexWindowRow: React.FC<{
  label: string;
  w: OpenAICodexUsageWindow;
  limitReached?: boolean;
}> = ({ label, w, limitReached }) => {
  const pct = clampPercent(w.used_percent);
  const windowText = formatLimitWindowSeconds(w.limit_window_seconds);
  const meta = formatQuotaMeta([
    formatUsagePercent(pct),
    windowText ? `Window ${windowText}` : null,
    formatResetAfterSeconds(w.reset_after_seconds),
    formatResetAt(w.reset_at),
  ]);
  return (
    <div className={styles.usageRow}>
      <div className={styles.usageRowHeader}>
        <span className={styles.usageLabelGroup}>
          {label}
          {limitReached ? <Badge tone="danger">Limit reached</Badge> : null}
        </span>
        <span>{meta}</span>
      </div>
      <UsageBar pct={pct} />
    </div>
  );
};

type ClaudeUsageWindowKey = keyof Pick<
  ClaudeCodeUsageData,
  | "five_hour"
  | "seven_day"
  | "seven_day_sonnet"
  | "seven_day_oauth_apps"
  | "seven_day_opus"
  | "seven_day_cowork"
  | "seven_day_omelette"
>;

const CLAUDE_USAGE_WINDOWS: {
  key: ClaudeUsageWindowKey;
  label: string;
}[] = [
  { key: "five_hour", label: "Current session" },
  { key: "seven_day", label: "Current week — all models" },
  { key: "seven_day_sonnet", label: "Current week — Sonnet" },
  { key: "seven_day_opus", label: "Current week — Opus" },
  { key: "seven_day_oauth_apps", label: "Current week — OAuth apps" },
  { key: "seven_day_cowork", label: "Current week — cowork" },
  { key: "seven_day_omelette", label: "Current week — Omelette" },
];

const InfoRow: React.FC<{ label: string; value: string }> = ({
  label,
  value,
}) => (
  <div className={styles.usageRowHeader}>
    <span>{label}</span>
    <span>{value}</span>
  </div>
);

const ClaudeUsagePanel: React.FC<{ data: ClaudeCodeUsageData }> = ({
  data,
}) => {
  const windowRows = CLAUDE_USAGE_WINDOWS.map(({ key, label }) => ({
    key,
    label,
    window: data[key],
  })).filter(
    (
      row,
    ): row is {
      key: ClaudeUsageWindowKey;
      label: string;
      window: ClaudeCodeUsageWindow;
    } => Boolean(row.window),
  );

  return (
    <Surface className={styles.usagePanel} variant="glass" animated="rise">
      <div className={styles.usageTitle}>Usage</div>
      <div className={styles.usageRows}>
        {windowRows.length > 0 ? (
          windowRows.map(({ key, label, window }) => (
            <ClaudeWindowRow key={key} label={label} w={window} />
          ))
        ) : (
          <div className={styles.usageMeta}>Quota windows not reported.</div>
        )}
        {data.extra_usage ? (
          <div className={styles.usageRow}>
            <div className={styles.usageRowHeader}>
              <span>Extra usage</span>
              <span>{formatClaudeExtraUsage(data.extra_usage)}</span>
            </div>
            {typeof data.extra_usage.utilization === "number" ? (
              <UsageBar pct={clampPercent(data.extra_usage.utilization)} />
            ) : null}
          </div>
        ) : (
          <div className={styles.usageMeta}>Extra usage not reported.</div>
        )}
      </div>
    </Surface>
  );
};

const RateLimitSection: React.FC<{
  title: string;
  rl: OpenAICodexRateLimit | null | undefined;
}> = ({ title, rl }) => {
  if (!rl) {
    return <div className={styles.usageMeta}>{title}: not reported.</div>;
  }

  const hasWindows = Boolean(rl.primary_window ?? rl.secondary_window);

  return (
    <div className={styles.usageRow}>
      <div className={styles.usageRowHeader}>
        <span className={styles.usageLabelGroup}>
          {title}
          {rl.limit_reached ? <Badge tone="danger">Limit reached</Badge> : null}
        </span>
        <span>
          {formatQuotaMeta([
            `allowed ${formatNullableBool(rl.allowed)}`,
            `limit reached ${formatNullableBool(rl.limit_reached)}`,
          ])}
        </span>
      </div>
      {rl.primary_window ? (
        <CodexWindowRow
          label={formatWindowLabel(
            "Primary",
            rl.primary_window.limit_window_seconds,
          )}
          w={rl.primary_window}
          limitReached={rl.limit_reached}
        />
      ) : null}
      {rl.secondary_window ? (
        <CodexWindowRow
          label={formatWindowLabel(
            "Secondary",
            rl.secondary_window.limit_window_seconds,
          )}
          w={rl.secondary_window}
        />
      ) : null}
      {!hasWindows ? (
        <div className={styles.usageMeta}>No active windows reported.</div>
      ) : null}
    </div>
  );
};

const AdditionalRateLimitRow: React.FC<{
  limit: OpenAICodexAdditionalRateLimit;
}> = ({ limit }) => (
  <div className={styles.usageRow}>
    <div className={styles.usageRowHeader}>
      <span>{limit.limit_name ?? "Additional quota"}</span>
      {limit.metered_feature ? <span>{limit.metered_feature}</span> : null}
    </div>
    <RateLimitSection title="Quota" rl={limit.rate_limit} />
  </div>
);

const formatRedeemCode = (code: string): string => {
  switch (code) {
    case "reset":
      return "Usage reset.";
    case "already_redeemed":
      return "Already redeemed.";
    case "nothing_to_reset":
      return "Your usage does not need a reset right now.";
    case "no_credit":
      return "No rate-limit resets are available.";
    default:
      return "Reset request completed.";
  }
};

const CodexUsagePanel: React.FC<{
  data: OpenAICodexUsageData;
  providerName: string;
  onRedeemed: () => void;
}> = ({ data, providerName, onRedeemed }) => {
  const [redeem, { isLoading: isRedeeming }] =
    useRedeemOpenAICodexResetCreditMutation();
  const [redeemMessage, setRedeemMessage] = React.useState<string | null>(null);
  const redeemRequestIdRef = React.useRef<string | null>(null);
  const availableResets = data.rate_limit_reset_credits?.available_count;
  const showResetCredits = typeof availableResets === "number";

  const handleRedeem = async () => {
    if (!redeemRequestIdRef.current) {
      redeemRequestIdRef.current = uuidv4();
    }
    setRedeemMessage(null);
    const response = await redeem({
      providerName,
      redeemRequestId: redeemRequestIdRef.current,
      useInstanceRoute: true,
    });
    if ("error" in response) {
      setRedeemMessage("Couldn't redeem reset. Please try again.");
      return;
    }
    const payload = response.data;
    if (payload.error != null || !payload.data) {
      setRedeemMessage(
        payload.error ?? "Couldn't redeem reset. Please try again.",
      );
      return;
    }
    // Idempotency key is reused on retry and only cleared after success.
    redeemRequestIdRef.current = null;
    setRedeemMessage(formatRedeemCode(payload.data.code));
    onRedeemed();
  };

  return (
    <Surface className={styles.usagePanel} variant="glass" animated="rise">
      <div className={styles.usageHeader}>
        <div>
          <div className={styles.usageTitle}>Usage</div>
          {data.email ? (
            <div className={styles.usageMeta}>{data.email}</div>
          ) : null}
        </div>
        {data.plan_type ? <Badge tone="accent">{data.plan_type}</Badge> : null}
      </div>
      <div className={styles.usageRows}>
        <RateLimitSection title="Main quota" rl={data.rate_limit} />
        {data.rate_limit_reached_type ? (
          <InfoRow label="Reached type" value={data.rate_limit_reached_type} />
        ) : null}
        {data.additional_rate_limits?.length ? (
          <div className={styles.usageRow}>
            <div className={styles.usageTitle}>Additional quotas</div>
            {data.additional_rate_limits.map((limit, index) => (
              <AdditionalRateLimitRow
                key={`${limit.limit_name ?? "quota"}-${index}`}
                limit={limit}
              />
            ))}
          </div>
        ) : (
          <div className={styles.usageMeta}>
            Additional quotas not reported.
          </div>
        )}
        <RateLimitSection
          title="Code review quota"
          rl={data.code_review_rate_limit}
        />
        {data.credits ? (
          <div className={styles.usageRow}>
            <InfoRow
              label="Credits"
              value={formatCodexCreditsSummary(data.credits)}
            />
            {formatCodexCreditsDetails(data.credits) ? (
              <div className={styles.usageMeta}>
                {formatCodexCreditsDetails(data.credits)}
              </div>
            ) : null}
          </div>
        ) : (
          <div className={styles.usageMeta}>Credits not reported.</div>
        )}
        {showResetCredits ? (
          <div className={styles.usageRow}>
            <div className={styles.resetCreditsRow}>
              <InfoRow
                label="Reset credits"
                value={`${availableResets} available`}
              />
              <Button
                size="1"
                variant="soft"
                loading={isRedeeming}
                disabled={isRedeeming || availableResets <= 0}
                onClick={() => void handleRedeem()}
              >
                Redeem reset
              </Button>
            </div>
            {redeemMessage ? (
              <div className={styles.usageMeta}>{redeemMessage}</div>
            ) : null}
          </div>
        ) : null}
        {data.spend_control ? (
          <InfoRow
            label="Spend control"
            value={formatCodexSpendControl(data.spend_control)}
          />
        ) : null}
      </div>
    </Surface>
  );
};

export const ProviderForm: React.FC<ProviderFormProps> = ({
  currentProvider,
}) => {
  const baseProvider = currentProvider.base_provider;
  const { data: openRouterHealth } = useGetOpenRouterHealthQuery(
    { providerName: currentProvider.name, useInstanceRoute: true },
    { skip: baseProvider !== "openrouter" },
  );
  const { data: claudeUsage, isError: claudeUsageError } =
    useGetClaudeCodeUsageQuery(
      { providerName: currentProvider.name, useInstanceRoute: true },
      { skip: baseProvider !== "claude_code", pollingInterval: 60_000 },
    );
  const {
    data: codexUsage,
    isError: codexUsageError,
    refetch: refetchCodexUsage,
  } = useGetOpenAICodexUsageQuery(
    { providerName: currentProvider.name, useInstanceRoute: true },
    { skip: baseProvider !== "openai_codex", pollingInterval: 60_000 },
  );
  const {
    areShowingExtraFields,
    formValues,
    parsedSchema,
    importantFields,
    extraFields,
    isProviderLoadedSuccessfully,
    setAreShowingExtraFields,
    handleFieldSave,
    detailedProvider,
  } = useProviderForm({ providerName: currentProvider.name });

  if (!isProviderLoadedSuccessfully || !formValues || !parsedSchema) {
    return <Spinner spinning />;
  }

  const hasOAuth = parsedSchema.oauth?.supported === true;
  const status: ProviderStatus =
    detailedProvider?.status ?? currentProvider.status;
  const hasCredentials =
    detailedProvider?.has_credentials ?? currentProvider.has_credentials;
  const isReadonly = formValues.readonly;

  return (
    <div className={styles.providerForm}>
      <div className={styles.formSection}>
        <div className={styles.statusRow}>
          <StatusBadge status={status} />
          {baseProvider === "openrouter" && openRouterHealth ? (
            <Badge tone={openRouterHealth.ok ? "success" : "danger"}>
              {openRouterHealth.ok ? "Key OK" : "Key Error"}
            </Badge>
          ) : null}
          {parsedSchema.description ? (
            <div className={styles.providerDescription}>
              {parsedSchema.description.trim().split("\n")[0]}
            </div>
          ) : null}
        </div>

        {claudeUsage?.data && !claudeUsage.error ? (
          <ClaudeUsagePanel data={claudeUsage.data} />
        ) : null}
        {claudeUsage?.error != null || claudeUsageError ? (
          <div className={styles.defaultDescription}>
            Usage: {claudeUsage?.error ?? "Failed to load"}
          </div>
        ) : null}

        {codexUsage?.data && !codexUsage.error ? (
          <CodexUsagePanel
            data={codexUsage.data}
            providerName={currentProvider.name}
            onRedeemed={() => void refetchCodexUsage()}
          />
        ) : null}
        {codexUsage?.error != null || codexUsageError ? (
          <div className={styles.defaultDescription}>
            Usage: {codexUsage?.error ?? "Failed to load"}
          </div>
        ) : null}

        <div className={styles.formSection}>
          {hasOAuth ? (
            <>
              <ProviderOAuth
                providerName={currentProvider.name}
                baseProvider={baseProvider}
                oauthConnected={Boolean(
                  "oauth_connected" in formValues && formValues.oauth_connected,
                )}
                authStatus={
                  "auth_status" in formValues
                    ? String(formValues.auth_status)
                    : ""
                }
              />
            </>
          ) : null}

          <div className={`${styles.formFields} rf-stagger`}>
            {importantFields.map((field) => (
              <div key={field.key} className="rf-enter-rise">
                <SchemaField
                  field={field}
                  value={formValues[field.key]}
                  disabled={isReadonly}
                  onSave={handleFieldSave}
                />
              </div>
            ))}
          </div>

          {extraFields.length > 0 ? (
            <>
              <div className={styles.advancedToggleWrap}>
                <Button
                  className={styles.extraButton}
                  variant="ghost"
                  size="sm"
                  onClick={() => setAreShowingExtraFields((prev) => !prev)}
                >
                  {areShowingExtraFields ? "Hide" : "Show"} advanced fields
                </Button>
              </div>

              {areShowingExtraFields ? (
                <div className={`${styles.formFields} rf-stagger`}>
                  {extraFields.map((field) => (
                    <div key={field.key} className="rf-enter-rise">
                      <SchemaField
                        field={field}
                        value={formValues[field.key]}
                        disabled={isReadonly}
                        onSave={handleFieldSave}
                      />
                    </div>
                  ))}
                </div>
              ) : null}
            </>
          ) : null}
        </div>

        {hasCredentials ? (
          <ProviderModelsList provider={currentProvider} />
        ) : null}

        {hasCredentials && baseProvider === "custom" && detailedProvider ? (
          <RoleSeparatedModelConfig provider={detailedProvider} />
        ) : null}
      </div>
    </div>
  );
};
