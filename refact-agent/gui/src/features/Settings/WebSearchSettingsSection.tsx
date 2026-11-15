import React, { useState } from "react";
import {
  Badge,
  Button,
  FieldText,
  SettingItem,
  Switch,
} from "../../components/ui";
import { SchemaField } from "../Providers/ProviderForm/SchemaField";
import { SettingsGroup, SettingsSection } from "./SettingsSection";
import {
  useGetWebSearchSettingsQuery,
  useUpdateWebSearchSettingsMutation,
  useTestWebSearchProvidersMutation,
  type ProviderReport,
  type WebSearchConfigPatch,
  type WebSearchProvider,
} from "../../services/refact/webSearchSettings";
import styles from "./WebSearchSettingsSection.module.css";

function errorMessage(error: unknown): string {
  if (error && typeof error === "object" && "data" in error) {
    const data = error.data;
    if (
      data &&
      typeof data === "object" &&
      "detail" in data &&
      typeof data.detail === "string"
    )
      return data.detail;
  }
  return "The request failed.";
}

function safeUrl(url: string | null): string | undefined {
  if (!url) return undefined;
  try {
    return ["http:", "https:"].includes(new URL(url).protocol)
      ? url
      : undefined;
  } catch {
    return undefined;
  }
}

function keyPlaceholder(provider: WebSearchProvider): string {
  if (provider.credential_status === "stored") return "•••• stored";
  if (provider.credential_status === "env")
    return `using ${provider.key_env_vars.join(" / ")} from environment`;
  return provider.key === "required" ? "required" : "Optional API key";
}

const GROUPS = [
  ["general", "General"],
  ["fallback", "Fallback"],
  ["specialist", "Specialist sources"],
] as const;

export const WebSearchSettingsSection: React.FC = () => {
  const {
    data: cachedData,
    currentData,
    error,
    isLoading,
    refetch,
  } = useGetWebSearchSettingsQuery(undefined);
  const data = currentData ?? cachedData;
  const [updateSettings, saveState] = useUpdateWebSearchSettingsMutation();
  const [testProviders, testState] = useTestWebSearchProvidersMutation();
  const [patch, setPatch] = useState<WebSearchConfigPatch>({});
  const [revision, setRevision] = useState(0);
  const [query, setQuery] = useState("");
  const [reports, setReports] = useState<
    Partial<Record<string, ProviderReport>>
  >({});
  const [requestError, setRequestError] = useState<string | null>(null);
  const changes: WebSearchConfigPatch = {};
  if (data) {
    if (
      patch.deadline_ms !== undefined &&
      patch.deadline_ms !== data.config.deadline_ms
    )
      changes.deadline_ms = patch.deadline_ms;
    if (
      patch.provider_timeout_ms !== undefined &&
      patch.provider_timeout_ms !== data.config.provider_timeout_ms
    )
      changes.provider_timeout_ms = patch.provider_timeout_ms;
    if (
      patch.prefer_builtin_over_native !== undefined &&
      patch.prefer_builtin_over_native !==
        data.config.prefer_builtin_over_native
    )
      changes.prefer_builtin_over_native = patch.prefer_builtin_over_native;
    if (
      patch.searxng_url !== undefined &&
      patch.searxng_url !== data.config.searxng_url
    )
      changes.searxng_url = patch.searxng_url;
    for (const [id, value] of Object.entries(patch.providers ?? {})) {
      if (!value) continue;
      const saved = data.config.providers[id];
      const provider: { enabled?: boolean; api_key?: string } = {};
      if (value.enabled !== undefined && value.enabled !== saved?.enabled)
        provider.enabled = value.enabled;
      if (value.api_key !== undefined && value.api_key !== saved?.api_key)
        provider.api_key = value.api_key;
      if (Object.keys(provider).length > 0) {
        changes.providers ??= {};
        changes.providers[id] = provider;
      }
    }
  }
  const dirty = Object.keys(changes).length > 0;

  const changeProvider = (
    id: string,
    value: { enabled?: boolean; api_key?: string },
  ) => {
    setPatch((current) => ({
      ...current,
      providers: {
        ...current.providers,
        [id]: { ...current.providers?.[id], ...value },
      },
    }));
  };
  const save = async () => {
    setRequestError(null);
    const candidate = data ? { ...data.config, ...changes } : null;
    if (
      !candidate ||
      !Number.isInteger(candidate.deadline_ms) ||
      candidate.deadline_ms < 3000 ||
      candidate.deadline_ms > 60000 ||
      !Number.isInteger(candidate.provider_timeout_ms) ||
      candidate.provider_timeout_ms < 1000 ||
      candidate.provider_timeout_ms > 30000
    ) {
      setRequestError(
        "Deadline must be 3000–60000 ms and provider timeout 1000–30000 ms.",
      );
      return;
    }
    if (candidate.searxng_url && !safeUrl(candidate.searxng_url)) {
      setRequestError("SearXNG URL must use HTTP or HTTPS.");
      return;
    }
    try {
      await updateSettings({ config: changes }).unwrap();
      setPatch({});
      setRevision((value) => value + 1);
    } catch (failure) {
      setRequestError(errorMessage(failure));
    }
  };
  const test = async (providers?: string[]) => {
    setRequestError(null);
    try {
      const result = await testProviders({
        ...(query.trim() ? { query: query.trim() } : {}),
        ...(providers ? { providers } : {}),
      }).unwrap();
      setReports((current) => ({
        ...current,
        ...Object.fromEntries(
          result.reports.map((report) => [report.provider, report]),
        ),
      }));
    } catch (failure) {
      setRequestError(errorMessage(failure));
    }
  };

  if (isLoading) return <p>Loading Web Search settings…</p>;
  if (!data)
    return (
      <div role="alert">
        <p>Could not load Web Search settings: {errorMessage(error)}</p>
        <Button onClick={() => void refetch()}>Retry</Button>
      </div>
    );
  const config = { ...data.config, ...patch };

  return (
    <SettingsSection
      title="Web Search"
      description="Fuse search providers and manage credentials. Settings apply to every search."
      width="wide"
      actions={
        <div className={styles.actions}>
          <Button
            disabled={!dirty || saveState.isLoading}
            onClick={() => {
              setPatch({});
              setRevision((value) => value + 1);
              setRequestError(null);
            }}
          >
            Reset
          </Button>
          <Button
            variant="primary"
            disabled={!dirty || saveState.isLoading}
            loading={saveState.isLoading}
            onClick={() => void save()}
          >
            Save
          </Button>
        </div>
      }
    >
      {requestError && (
        <p className={styles.error} role="alert">
          {requestError}
        </p>
      )}
      <fieldset className={styles.fields} disabled={saveState.isLoading}>
        <SettingsGroup title="Global settings">
          <SettingItem
            title="Prefer Refact search over model-native search"
            description="Keep the fused Refact tool instead of the model's native search."
            control={
              <Switch
                aria-label="Prefer Refact search over model-native search"
                checked={config.prefer_builtin_over_native}
                onCheckedChange={(value) =>
                  setPatch((current) => ({
                    ...current,
                    prefer_builtin_over_native: value,
                  }))
                }
              />
            }
          />
          {(
            [
              ["deadline_ms", "Search deadline (ms)", 3000, 60000],
              ["provider_timeout_ms", "Provider timeout (ms)", 1000, 30000],
            ] as const
          ).map(([key, label, min, max]) => (
            <SettingItem
              key={key}
              title={label}
              control={
                <FieldText
                  aria-label={label}
                  type="number"
                  min={min}
                  max={max}
                  step={1}
                  value={String(config[key])}
                  onChange={(value) =>
                    setPatch((current) => ({
                      ...current,
                      [key]: Number(value),
                    }))
                  }
                />
              }
            />
          ))}
          <SettingItem
            title="SearXNG URL"
            description="Optional self-hosted HTTP or HTTPS endpoint."
            control={
              <FieldText
                aria-label="SearXNG URL"
                type="url"
                value={config.searxng_url}
                onChange={(value) =>
                  setPatch((current) => ({ ...current, searxng_url: value }))
                }
              />
            }
          />
        </SettingsGroup>
        {GROUPS.map(([tier, title]) => (
          <SettingsGroup key={tier} title={title}>
            {data.providers
              .filter((provider) => provider.tier === tier)
              .map((provider) => {
                const signupUrl = safeUrl(provider.signup_url);
                const lastReport = reports[provider.id];
                const providerPatch = patch.providers?.[provider.id];
                return (
                  <div
                    className={styles.provider}
                    key={`${provider.id}-${revision}`}
                  >
                    <SettingItem
                      title={provider.display_name}
                      description={provider.description}
                      control={
                        <Switch
                          aria-label={`Enable ${provider.display_name}`}
                          checked={
                            providerPatch?.enabled ??
                            data.config.providers[provider.id]?.enabled ??
                            provider.default_enabled
                          }
                          onCheckedChange={(enabled) =>
                            changeProvider(provider.id, { enabled })
                          }
                        />
                      }
                    />
                    <div className={styles.actions}>
                      {provider.category && <Badge>{provider.category}</Badge>}
                      <Badge>{provider.active ? "active" : "inactive"}</Badge>
                      {lastReport && <Badge>{lastReport.status}</Badge>}
                      {signupUrl && (
                        <a
                          href={signupUrl}
                          target="_blank"
                          rel="noopener noreferrer"
                        >
                          Get API key
                        </a>
                      )}
                      <Button
                        disabled={dirty || testState.isLoading}
                        onClick={() => void test([provider.id])}
                        aria-label={`Test ${provider.display_name}`}
                      >
                        Test
                      </Button>
                    </div>
                    {provider.key !== "none" && (
                      <SchemaField
                        field={{
                          key: `${provider.id}_api_key`,
                          f_type: "string",
                          f_secret: true,
                          f_label: `${provider.display_name} API key`,
                          f_placeholder: keyPlaceholder(provider),
                        }}
                        value={
                          data.config.providers[provider.id]?.api_key ?? ""
                        }
                        onDraftChange={(api_key) =>
                          changeProvider(provider.id, {
                            api_key:
                              api_key === "" &&
                              data.config.providers[provider.id]?.api_key ===
                                "***"
                                ? "***"
                                : api_key,
                          })
                        }
                        onSave={(_key, value) => {
                          if (typeof value === "string")
                            changeProvider(provider.id, { api_key: value });
                          return Promise.resolve();
                        }}
                      />
                    )}
                  </div>
                );
              })}
          </SettingsGroup>
        ))}
      </fieldset>
      <SettingsGroup
        title="Test providers"
        description="Tests use saved settings. Save or reset changes before testing."
      >
        <div className={styles.actions}>
          <FieldText
            aria-label="Test query"
            placeholder="rust tokio select cancellation safety"
            value={query}
            onChange={setQuery}
          />
          <Button
            disabled={dirty || testState.isLoading}
            loading={testState.isLoading}
            onClick={() => void test()}
          >
            Test providers
          </Button>
        </div>
        {Object.keys(reports).length > 0 && (
          <div className={styles.tableScroll}>
            <table className={styles.table}>
              <thead>
                <tr>
                  {[
                    "Provider",
                    "Status",
                    "Results",
                    "Latency ms",
                    "Detail",
                    "Top result",
                  ].map((label) => (
                    <th key={label}>{label}</th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {Object.values(reports)
                  .filter(
                    (report): report is ProviderReport => report !== undefined,
                  )
                  .map((report) => (
                    <tr key={report.provider}>
                      <td>
                        {data.providers.find(
                          (provider) => provider.id === report.provider,
                        )?.display_name ?? report.provider}
                      </td>
                      <td>
                        <Badge>{report.status}</Badge>
                      </td>
                      <td>{report.results}</td>
                      <td>{report.latency_ms}</td>
                      <td>{report.detail}</td>
                      <td>
                        {report.top[0] && safeUrl(report.top[0].url) && (
                          <a
                            href={report.top[0].url}
                            target="_blank"
                            rel="noopener noreferrer"
                          >
                            {report.top[0].title}
                          </a>
                        )}
                      </td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </div>
        )}
      </SettingsGroup>
    </SettingsSection>
  );
};
