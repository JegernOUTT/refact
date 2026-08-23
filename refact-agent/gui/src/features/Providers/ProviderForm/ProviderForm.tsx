import React from "react";
import { ChevronDown } from "lucide-react";
import { v4 as uuidv4 } from "uuid";

import { SchemaField } from "./SchemaField";
import { ProviderOAuth } from "./ProviderOAuth";
import { Spinner } from "../../../components/Spinner";

import { useProviderForm } from "./useProviderForm";
import type {
  ProviderListItem,
  ProviderQuotaFact,
  ProviderQuotaSnapshot,
  ProviderQuotaWindow,
  ProviderStatus,
} from "../../../services/refact";
import { Badge, Button, ProviderQuota, Surface } from "../../../components/ui";

import styles from "./ProviderForm.module.css";
import { ProviderModelsList } from "./ProviderModelsList/ProviderModelsList";
import {
  useGetOpenRouterHealthQuery,
  useGetProviderQuotaQuery,
  useRedeemOpenAICodexResetCreditMutation,
} from "../../../services/refact";
import {
  formatLimitWindowSeconds,
  formatQuotaMeta,
  formatResetAfterSeconds,
  formatResetAt,
  formatUsagePercent,
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

const formatNumber = (value: number): string =>
  value.toLocaleString(undefined, { maximumFractionDigits: 2 });

const formatFactValue = (fact: ProviderQuotaFact): string => {
  if (fact.value === null) return "Unavailable";
  const value =
    typeof fact.value === "boolean"
      ? fact.value
        ? "Yes"
        : "No"
      : typeof fact.value === "number"
        ? formatNumber(fact.value)
        : fact.value;
  return fact.unit ? `${value} ${fact.unit}` : String(value);
};

const formatWindowValue = (window: ProviderQuotaWindow): string => {
  if (typeof window.used === "number" && typeof window.limit === "number") {
    return `${formatNumber(window.used)} / ${formatNumber(window.limit)}`;
  }
  if (
    typeof window.remaining === "number" &&
    typeof window.limit === "number"
  ) {
    return `${formatNumber(window.remaining)} / ${formatNumber(
      window.limit,
    )} remaining`;
  }
  return typeof window.used_percent === "number"
    ? formatUsagePercent(window.used_percent)
    : "Usage unavailable";
};

const formatWindowMeta = (window: ProviderQuotaWindow): string | undefined => {
  const hasCountValue =
    (typeof window.used === "number" || typeof window.remaining === "number") &&
    typeof window.limit === "number";
  const duration = formatLimitWindowSeconds(window.window_seconds);
  const meta = formatQuotaMeta([
    hasCountValue && typeof window.used_percent === "number"
      ? formatUsagePercent(window.used_percent)
      : null,
    duration ? `Window ${duration}` : null,
    formatResetAfterSeconds(window.reset_after_seconds),
    formatResetAt(window.reset_at),
  ]);
  return meta || undefined;
};

const quotaTone = (
  window: ProviderQuotaWindow,
): "accent" | "warning" | "danger" => {
  const status = window.status?.toLocaleLowerCase() ?? "";
  if (status.includes("limit") || status.includes("error")) return "danger";
  if (status.includes("warning") || (window.used_percent ?? 0) >= 80) {
    return "warning";
  }
  return "accent";
};

const findResetCredits = (facts: ProviderQuotaFact[]): number | null => {
  const fact = facts.find((candidate) => {
    const identity = `${candidate.id} ${candidate.label}`.toLocaleLowerCase();
    return identity.includes("reset") && identity.includes("credit");
  });
  return typeof fact?.value === "number" ? fact.value : null;
};

const CodexResetCreditAction: React.FC<{
  availableResets: number;
  providerName: string;
  onRedeemed: () => void;
}> = ({ availableResets, providerName, onRedeemed }) => {
  const [redeem, { isLoading: isRedeeming }] =
    useRedeemOpenAICodexResetCreditMutation();
  const [redeemMessage, setRedeemMessage] = React.useState<string | null>(null);
  const redeemRequestIdRef = React.useRef<string | null>(null);

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
    redeemRequestIdRef.current = null;
    setRedeemMessage(formatRedeemCode(payload.data.code));
    onRedeemed();
  };

  return (
    <div className={styles.usageActions}>
      <Button
        size="1"
        variant="soft"
        loading={isRedeeming}
        disabled={isRedeeming || availableResets <= 0}
        onClick={() => void handleRedeem()}
      >
        Redeem reset
      </Button>
      {redeemMessage ? (
        <div className={styles.usageMeta} role="status">
          {redeemMessage}
        </div>
      ) : null}
    </div>
  );
};

type ProviderQuotaPanelProps = {
  action?: React.ReactNode;
  error: boolean;
  fetching: boolean;
  loading: boolean;
  snapshot?: ProviderQuotaSnapshot;
};

export const ProviderQuotaPanel: React.FC<ProviderQuotaPanelProps> = ({
  action,
  error,
  fetching,
  loading,
  snapshot,
}) => {
  const fetchedAt = snapshot?.fetched_at
    ? new Date(snapshot.fetched_at).toLocaleString()
    : null;

  return (
    <Surface className={styles.usagePanel} variant="glass" animated="rise">
      <div className={styles.usageHeader}>
        <div>
          <div className={styles.usageTitle}>Quota</div>
          {snapshot ? (
            <div className={styles.usageMeta}>
              {formatQuotaMeta([
                `Source: ${snapshot.source}`,
                fetchedAt && fetchedAt !== "Invalid Date"
                  ? `Updated ${fetchedAt}`
                  : null,
              ])}
            </div>
          ) : null}
        </div>
        <div className={styles.usageBadges}>
          {fetching && !loading ? (
            <Badge tone="accent">Refreshing</Badge>
          ) : null}
          {snapshot?.stale ? <Badge tone="warning">Stale</Badge> : null}
          {snapshot && !snapshot.available ? (
            <Badge tone="muted">Unavailable</Badge>
          ) : null}
        </div>
      </div>

      {loading ? <div className={styles.usageMeta}>Loading quota…</div> : null}
      {error ? (
        <div className={styles.usageMeta} role="alert">
          {snapshot ? "Failed to refresh quota." : "Failed to load quota."}
        </div>
      ) : null}
      {snapshot?.error ? (
        <div className={styles.usageMeta} role="alert">
          {snapshot.error}
        </div>
      ) : null}
      {snapshot && !snapshot.available && !snapshot.error ? (
        <div className={styles.usageMeta}>
          Quota information is unavailable.
        </div>
      ) : null}

      {snapshot?.available ? (
        <div className={styles.usageRows}>
          {snapshot.windows.map((window) => (
            <ProviderQuota
              key={window.id}
              label={window.label}
              value={formatWindowValue(window)}
              meta={formatWindowMeta(window)}
              usedPercent={window.used_percent ?? undefined}
              tone={quotaTone(window)}
              badge={
                window.status ? (
                  <Badge
                    tone={quotaTone(window) === "danger" ? "danger" : "muted"}
                  >
                    {window.status}
                  </Badge>
                ) : undefined
              }
            />
          ))}
          {snapshot.facts.map((fact) => (
            <ProviderQuota
              key={fact.id}
              label={fact.label}
              value={formatFactValue(fact)}
              tone="muted"
            />
          ))}
          {snapshot.windows.length === 0 && snapshot.facts.length === 0 ? (
            <div className={styles.usageMeta}>
              No quota windows or account facts were reported.
            </div>
          ) : null}
        </div>
      ) : null}
      {action}
    </Surface>
  );
};
export const ProviderForm: React.FC<ProviderFormProps> = ({
  currentProvider,
}) => {
  const baseProvider = currentProvider.base_provider;
  const [forceQuotaRefresh, setForceQuotaRefresh] = React.useState(false);
  const { data: openRouterHealth } = useGetOpenRouterHealthQuery(
    { providerName: currentProvider.name, useInstanceRoute: true },
    { skip: baseProvider !== "openrouter" },
  );
  const {
    data: quotaResponse,
    isError: quotaError,
    isFetching: quotaFetching,
    isLoading: quotaLoading,
  } = useGetProviderQuotaQuery(
    { providerName: currentProvider.name, refresh: forceQuotaRefresh },
    { pollingInterval: 60_000 },
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

        <ProviderQuotaPanel
          snapshot={quotaResponse?.quota}
          loading={quotaLoading}
          fetching={quotaFetching}
          error={quotaError}
          action={
            baseProvider === "openai_codex" && quotaResponse?.quota
              ? (() => {
                  const availableResets = findResetCredits(
                    quotaResponse.quota.facts,
                  );
                  return availableResets === null ? null : (
                    <CodexResetCreditAction
                      availableResets={availableResets}
                      providerName={currentProvider.name}
                      onRedeemed={() => setForceQuotaRefresh(true)}
                    />
                  );
                })()
              : null
          }
        />

        <div className={styles.formSection}>
          {hasOAuth ? (
            <>
              {parsedSchema.oauth?.warning ? (
                <div className={styles.oauthWarning} role="alert">
                  <span className={styles.oauthWarningIcon} aria-hidden="true">
                    ⚠️
                  </span>
                  <span>{parsedSchema.oauth.warning}</span>
                </div>
              ) : null}
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
                  rightIcon={ChevronDown}
                  aria-expanded={areShowingExtraFields}
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
      </div>
    </div>
  );
};
