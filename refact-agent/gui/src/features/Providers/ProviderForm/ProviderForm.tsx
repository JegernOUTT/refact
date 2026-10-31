import React from "react";
import { ChevronDown } from "lucide-react";
import { v4 as uuidv4 } from "uuid";

import { SchemaField } from "./SchemaField";
import { ProviderOAuth } from "./ProviderOAuth";
import { BedrockSetup } from "./BedrockSetup";
import { Spinner } from "../../../components/Spinner";

import { useProviderForm } from "./useProviderForm";
import {
  claudeLoginKind,
  effectiveClaudeAuthType,
  effectiveClaudeBackend,
  isOAuthLoginAvailable,
} from "./schemaConditions";
import type { ClaudeLoginKind } from "./schemaConditions";
import type {
  ClaudeCodeResetClaimData,
  ClaudeCodeResetGrant,
  ClaudeCodeSavedResets,
  ClaudeCodeSessionReset,
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
  useClaimClaudeCodeResetMutation,
  useGetClaudeCodeResetStatusQuery,
  useGetOpenRouterHealthQuery,
  useGetProviderQuotaQuery,
  useRedeemOpenAICodexResetCreditMutation,
} from "../../../services/refact";
import {
  formatClaudeLimitName,
  formatLimitWindowSeconds,
  formatQuotaMeta,
  formatQuotaMoment,
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

const humanizeResetReason = (reason: string): string =>
  reason.replace(/_/g, " ");

const formatClaudeResetClaim = (claim: ClaudeCodeResetClaimData): string => {
  switch (claim.result) {
    case "reset": {
      const cleared = claim.cleared.length
        ? ` Cleared: ${claim.cleared.map(formatClaudeLimitName).join(", ")}.`
        : "";
      const left =
        typeof claim.resets_left === "number"
          ? ` ${claim.resets_left} saved ${
              claim.resets_left === 1 ? "reset" : "resets"
            } left.`
          : "";
      const next = formatQuotaMoment(claim.next_available_at);
      return `Usage limit reset.${cleared}${left}${
        next ? ` Next session reset available ${next}.` : ""
      }`;
    }
    case "already_used":
      return claim.program === "juniper_tide"
        ? "Your session limit is already being reset."
        : "Another reset was just started on your account.";
    case "not_limited":
      return "Your reset only works at a usage limit, and you're not at one now.";
    case "cooldown": {
      const until = formatQuotaMoment(claim.cooldown_until);
      return until
        ? `Resets are cooling down until ${until}.`
        : "Resets are cooling down. Try again later.";
    }
    case "ineligible":
      return claim.reason
        ? `This reset can't be used right now (${humanizeResetReason(
            claim.reason,
          )}).`
        : "This reset can't be used right now.";
    default:
      return claim.program === "juniper_tide"
        ? "A session-limit reset isn't available right now."
        : "A reset isn't available right now.";
  }
};

const findNextClaudeGrant = (
  saved: ClaudeCodeSavedResets,
): ClaudeCodeResetGrant | null => {
  if (!saved.eligible) return null;
  const grant = saved.grants.find((item) => item.id === saved.next_grant_id);
  return grant ?? null;
};

const describeClaudeGrant = (
  grant: ClaudeCodeResetGrant,
  saved: ClaudeCodeSavedResets,
): string => {
  const parts: string[] = [];
  parts.push(
    `${grant.resets_left} of ${grant.resets_total} ${
      grant.resets_total === 1 ? "reset" : "resets"
    } left${grant.label ? ` · ${grant.label}` : ""}`,
  );
  if (grant.clears.length) {
    parts.push(`Clears ${grant.clears.map(formatClaudeLimitName).join(", ")}`);
  }
  const endsAt = formatQuotaMoment(grant.ends_at);
  if (endsAt) parts.push(`Expires ${endsAt}`);
  if (grant.paused) {
    parts.push("Paused");
  } else if (grant.use_requires_limit && !saved.at_limit) {
    parts.push("Works when you reach a usage limit");
  }
  return parts.join(" · ");
};

const describeClaudeSessionReset = (
  session: ClaudeCodeSessionReset,
): string => {
  if (session.available) {
    return `Clears the current session limit · ${session.resets_per_week} per week`;
  }
  const next = formatQuotaMoment(session.next_available_at);
  if (next) return `Next session reset available ${next}`;
  if (session.ineligible_reason) {
    return `Unavailable (${humanizeResetReason(session.ineligible_reason)})`;
  }
  return "No session reset available right now";
};

const ClaudeCodeResetAction: React.FC<{
  providerName: string;
  onClaimed: () => void;
}> = ({ providerName, onClaimed }) => {
  const {
    data: statusResponse,
    isFetching,
    isError,
    refetch,
  } = useGetClaudeCodeResetStatusQuery({
    providerName,
    useInstanceRoute: true,
  });
  const [claim, { isLoading: isClaiming }] = useClaimClaudeCodeResetMutation();
  const [message, setMessage] = React.useState<string | null>(null);
  const savedRequestIdRef = React.useRef<string | null>(null);

  const saved = statusResponse?.data?.cedar_ember ?? null;
  const session = statusResponse?.data?.juniper_tide ?? null;
  const nextGrant = saved ? findNextClaudeGrant(saved) : null;
  const sessionOffered = session?.eligible === true && session.arm === "reset";
  const statusError = isError
    ? "Couldn't check usage resets."
    : statusResponse?.error ?? null;
  const nothingOffered =
    !statusError && !isFetching && !nextGrant && !sessionOffered;
  const ineligibleReason =
    saved?.ineligible_reason ?? session?.ineligible_reason ?? null;

  const runClaim = async (program: "cedar_ember" | "juniper_tide") => {
    setMessage(null);
    if (program === "cedar_ember") {
      if (!nextGrant) return;
      if (!savedRequestIdRef.current) savedRequestIdRef.current = uuidv4();
    }
    const response = await claim({
      providerName,
      useInstanceRoute: true,
      program,
      grantId: program === "cedar_ember" ? nextGrant?.id : undefined,
      requestId:
        program === "cedar_ember"
          ? savedRequestIdRef.current ?? undefined
          : undefined,
    });
    if ("error" in response) {
      setMessage("Couldn't use the reset. Please try again.");
      return;
    }
    const payload = response.data;
    if (payload.error != null || !payload.data) {
      setMessage(payload.error ?? "Couldn't use the reset. Please try again.");
      return;
    }
    if (program === "cedar_ember") savedRequestIdRef.current = null;
    setMessage(formatClaudeResetClaim(payload.data));
    onClaimed();
  };

  return (
    <div className={styles.resetBlock} data-testid="claude-code-resets">
      <div className={styles.usageActions}>
        <div className={styles.usageTitle}>Usage resets</div>
        <Button
          size="1"
          variant="ghost"
          loading={isFetching}
          disabled={isFetching || isClaiming}
          onClick={() => void refetch()}
        >
          Refresh
        </Button>
      </div>
      {statusError ? (
        <div className={styles.usageMeta} role="alert">
          {statusError}
        </div>
      ) : null}
      {saved?.eligible && nextGrant ? (
        <div className={styles.usageActions}>
          <div className={styles.usageMeta}>
            {describeClaudeGrant(nextGrant, saved)}
          </div>
          <Button
            size="1"
            variant="soft"
            loading={isClaiming}
            disabled={
              isClaiming ||
              !nextGrant.usable_now ||
              nextGrant.paused ||
              nextGrant.resets_left <= 0
            }
            onClick={() => void runClaim("cedar_ember")}
          >
            Use saved reset
          </Button>
        </div>
      ) : null}
      {session && sessionOffered ? (
        <div className={styles.usageActions}>
          <div className={styles.usageMeta}>
            {describeClaudeSessionReset(session)}
          </div>
          <Button
            size="1"
            variant="soft"
            loading={isClaiming}
            disabled={isClaiming || !session.available}
            onClick={() => void runClaim("juniper_tide")}
          >
            Reset session limit
          </Button>
        </div>
      ) : null}
      {nothingOffered ? (
        <div className={styles.usageMeta}>
          {ineligibleReason
            ? `No usage resets are available for this account (${humanizeResetReason(
                ineligibleReason,
              )}).`
            : "No usage resets are available for this account right now."}
        </div>
      ) : null}
      {message ? (
        <div className={styles.usageMeta} role="status">
          {message}
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
const describeClaudeCredentialSources = (
  loginKind: ClaudeLoginKind | undefined,
  backend: string | undefined,
): string => {
  if (loginKind === "console") {
    return "Credentials are resolved by the engine, not the browser. Anthropic Console sign-in stores an OAuth credential for this provider instance on the engine host.";
  }
  if (backend === "bedrock") {
    return "Credentials are resolved by the engine, not the browser. Like Claude Code, Bedrock uses the engine host's AWS credential chain: environment credentials, the selected AWS profile (including SSO), or workload roles. AWS_BEARER_TOKEN_BEDROCK takes precedence over AWS credentials; explicit access keys saved here override the credential chain. AWS SSO Login is optional and only refreshes an already configured SSO profile. To create one, run aws configure sso --profile your-profile on the engine host, then enter that AWS Profile below. Add the Bedrock model or inference-profile ID under Available Models. Credentials and model access are checked on request, not on save.";
  }
  if (loginKind === "native") {
    return "Credentials are resolved by the engine, not the browser. Credential fields saved here win first, then engine environment variables, then the engine host's native credential store (AWS profiles and SSO cache, Google Application Default Credentials, or Azure CLI and managed identity). Signing in updates that native store on the engine host and only takes effect while explicit credentials are blank.";
  }
  return "Credentials are resolved by the engine, not the browser. Saved settings override engine environment variables.";
};

export const ProviderForm: React.FC<ProviderFormProps> = ({
  currentProvider,
}) => {
  const baseProvider = currentProvider.base_provider;
  const {
    areShowingExtraFields,
    formValues,
    parsedSchema,
    importantFields,
    extraFields,
    isProviderLoadedSuccessfully,
    setAreShowingExtraFields,
    handleFieldSave,
    handleSettingsSave,
    detailedProvider,
  } = useProviderForm({ providerName: currentProvider.name });
  const claudeBackend = formValues
    ? effectiveClaudeBackend(baseProvider, formValues)
    : undefined;
  const skipQuota =
    (baseProvider === "claude_code" || baseProvider === "anthropic") &&
    (!formValues || claudeBackend !== "oauth");
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
    { pollingInterval: 60_000, skip: skipQuota },
  );

  if (!isProviderLoadedSuccessfully || !formValues || !parsedSchema) {
    return <Spinner spinning />;
  }

  const loginKind = claudeLoginKind(
    baseProvider,
    claudeBackend,
    effectiveClaudeAuthType(formValues),
  );
  const hasOAuth =
    claudeBackend === undefined
      ? parsedSchema.oauth?.supported === true
      : loginKind !== undefined &&
        isOAuthLoginAvailable(
          parsedSchema.oauth,
          parsedSchema.fields,
          formValues,
        );
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
            <div className={styles.statusDescription}>
              {parsedSchema.description.trim().split("\n")[0]}
            </div>
          ) : null}
        </div>

        {!skipQuota ? (
          <ProviderQuotaPanel
            snapshot={quotaResponse?.quota}
            loading={quotaLoading}
            fetching={quotaFetching}
            error={quotaError}
            action={
              baseProvider === "claude_code" &&
              claudeBackend === "oauth" &&
              quotaResponse?.quota ? (
                <ClaudeCodeResetAction
                  providerName={currentProvider.name}
                  onClaimed={() => setForceQuotaRefresh(true)}
                />
              ) : baseProvider === "openai_codex" && quotaResponse?.quota ? (
                (() => {
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
              ) : null
            }
          />
        ) : null}

        <div className={styles.formSection}>
          {claudeBackend &&
          claudeBackend !== "oauth" &&
          claudeBackend !== "bedrock" ? (
            <div className={styles.credentialDescription}>
              {describeClaudeCredentialSources(loginKind, claudeBackend)}
            </div>
          ) : null}
          {claudeBackend === "bedrock" ? (
            <BedrockSetup
              key={currentProvider.name}
              providerName={currentProvider.name}
              readonly={isReadonly}
              settingsRevision={JSON.stringify(detailedProvider?.settings)}
              recoverySettingsRevision={JSON.stringify(
                Object.entries({
                  base_provider: detailedProvider?.base_provider,
                  display_name: detailedProvider?.display_name,
                  ...detailedProvider?.settings,
                })
                  .filter(
                    ([key]) => key !== "aws_profile" && key !== "aws_region",
                  )
                  .sort(([a], [b]) => a.localeCompare(b)),
              )}
              onSaveRecovery={async (profile, region) => {
                await handleSettingsSave({
                  aws_profile: profile,
                  aws_region: region,
                });
              }}
              savedProfile={
                typeof formValues.aws_profile === "string"
                  ? formValues.aws_profile
                  : undefined
              }
              savedRegion={
                typeof formValues.aws_region === "string"
                  ? formValues.aws_region
                  : undefined
              }
              recovery={
                hasOAuth ? (
                  <ProviderOAuth
                    providerName={currentProvider.name}
                    baseProvider={baseProvider}
                    loginKind={loginKind}
                    backend={claudeBackend}
                    readonly={isReadonly}
                    oauthConnected={Boolean(formValues.oauth_connected)}
                    authStatus={
                      typeof formValues.auth_status === "string"
                        ? formValues.auth_status
                        : ""
                    }
                  />
                ) : undefined
              }
            />
          ) : null}
          {hasOAuth && claudeBackend !== "bedrock" ? (
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
                key={`${currentProvider.name}:${claudeBackend ?? ""}:${
                  loginKind ?? ""
                }`}
                providerName={currentProvider.name}
                baseProvider={baseProvider}
                loginKind={loginKind}
                backend={claudeBackend}
                readonly={isReadonly}
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

        {hasCredentials ||
        (claudeBackend !== undefined && claudeBackend !== "oauth") ? (
          <ProviderModelsList provider={currentProvider} />
        ) : null}
      </div>
    </div>
  );
};
