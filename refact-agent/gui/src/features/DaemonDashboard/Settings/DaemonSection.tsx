import { useEffect, useMemo, useState } from "react";
import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";
import { Lock } from "lucide-react";

import {
  EmptyState,
  FieldError,
  FieldSwitch,
  FieldText,
  LoadingState,
  SettingItem,
  Surface,
} from "../../../components/ui";
import {
  type DaemonSettings,
  type DaemonSettingsUpdate,
  useGetDaemonSettingsQuery,
  useUpdateDaemonSettingsMutation,
} from "../../../services/refact/daemon";
import { SettingsGroup } from "../../Settings/SettingsSection";
import { QrCode } from "./QrCode";
import styles from "./SettingsPage.module.css";

type DaemonSettingsForm = {
  lan_enabled: boolean;
  mdns_enabled: boolean;
  auth_enabled: boolean;
  username: string;
  password: string;
};

function settingsToForm(settings: DaemonSettings): DaemonSettingsForm {
  return {
    lan_enabled: settings.lan_enabled,
    mdns_enabled: settings.mdns_enabled,
    auth_enabled: settings.auth_enabled,
    username: settings.username ?? "",
    password: "",
  };
}

function buildSettingsPayload(form: DaemonSettingsForm): DaemonSettingsUpdate {
  const payload: DaemonSettingsUpdate = {
    lan_enabled: form.lan_enabled,
    mdns_enabled: form.mdns_enabled,
    auth_enabled: form.auth_enabled,
  };
  const username = form.username.trim();
  if (username) payload.username = username;
  if (form.password) payload.password = form.password;
  return payload;
}

function errorMessage(error: unknown): string {
  const queryError = error as FetchBaseQueryError | undefined;
  if (queryError && "data" in queryError) {
    const data = queryError.data;
    if (typeof data === "object" && data !== null && "error" in data) {
      const message = (data as { error?: unknown }).error;
      if (typeof message === "string" && message) return message;
    }
  }
  return "Could not save daemon settings.";
}

export function DaemonSection() {
  const { data, isLoading } = useGetDaemonSettingsQuery(undefined);
  const [updateSettings, updateState] = useUpdateDaemonSettingsMutation();
  const [form, setForm] = useState<DaemonSettingsForm | null>(null);
  const [guardError, setGuardError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const settings = data?.settings ?? null;
  const authHidden = data?.access === "auth_hidden";

  useEffect(() => {
    if (settings) setForm(settingsToForm(settings));
  }, [settings]);

  const reachableUrls = useMemo(() => {
    if (!settings || !form?.lan_enabled) return [];
    return [settings.urls.mdns, settings.urls.loopback].filter(
      (url, index, urls) => url.trim() && urls.indexOf(url) === index,
    );
  }, [form?.lan_enabled, settings]);

  const qrUrl = settings
    ? settings.urls.mdns.trim() || settings.urls.loopback.trim()
    : "";

  function hasLanCredentials(next: DaemonSettingsForm): boolean {
    return (
      next.auth_enabled &&
      next.username.trim().length > 0 &&
      (next.password.length > 0 || settings?.has_password === true)
    );
  }

  async function save(next: DaemonSettingsForm) {
    setSaved(false);
    setSaveError(null);
    if (next.lan_enabled && !hasLanCredentials(next)) {
      setGuardError(
        "LAN access requires authentication with a username and password.",
      );
      return;
    }
    setGuardError(null);
    try {
      await updateSettings(buildSettingsPayload(next)).unwrap();
      setSaved(true);
    } catch (error) {
      setSaveError(errorMessage(error));
    }
  }

  function changeForm(patch: Partial<DaemonSettingsForm>) {
    setSaved(false);
    setGuardError(null);
    setSaveError(null);
    setForm((current) => (current ? { ...current, ...patch } : current));
  }

  if (isLoading || !data) {
    return <LoadingState kind="skeleton" label="Loading daemon settings" />;
  }

  if (authHidden) {
    return (
      <EmptyState
        icon={Lock}
        title="Daemon settings require authentication"
        description="Authenticate with the daemon before changing network access settings."
      />
    );
  }

  if (!form || !settings) return null;

  const saveStatus = updateState.isLoading
    ? "saving"
    : saveError
      ? "error"
      : saved
        ? "saved"
        : "idle";

  return (
    <SettingsGroup title="Daemon">
      <Surface className={styles.sectionSurface} variant="glass">
        <SettingItem
          title="Bind address"
          description="The active address reported by the daemon."
          control={<code className={styles.value}>{settings.bind}</code>}
        />
        <SettingItem
          title="Local network access"
          description="Listen beyond loopback. LAN access requires Basic-auth credentials."
          saveStatus={saveStatus}
          saveStatusLabel={saved ? "Restarting…" : undefined}
          control={
            <FieldSwitch
              aria-label="Local network access"
              checked={form.lan_enabled}
              onChange={(lan_enabled) =>
                changeForm({
                  lan_enabled,
                  auth_enabled: lan_enabled || form.auth_enabled,
                })
              }
              onCommit={(lan_enabled) =>
                void save({
                  ...form,
                  lan_enabled,
                  auth_enabled: lan_enabled || form.auth_enabled,
                })
              }
            />
          }
        />
        <SettingItem
          title="mDNS discovery"
          description="Advertise the daemon on the local network."
          control={
            <FieldSwitch
              aria-label="mDNS discovery"
              checked={form.mdns_enabled}
              onChange={(mdns_enabled) => changeForm({ mdns_enabled })}
              onCommit={(mdns_enabled) => void save({ ...form, mdns_enabled })}
            />
          }
        />
        <SettingItem
          title="Basic authentication"
          description="Require a username and password for protected daemon APIs."
          control={
            <FieldSwitch
              aria-label="Basic authentication"
              checked={form.auth_enabled}
              onChange={(auth_enabled) => changeForm({ auth_enabled })}
              onCommit={(auth_enabled) => void save({ ...form, auth_enabled })}
            />
          }
        />
        <SettingItem
          layout="stack"
          title="Username"
          description="Saved when the field loses focus."
          control={
            <FieldText
              aria-label="Basic-auth username"
              value={form.username}
              onChange={(username) => changeForm({ username })}
              onCommit={(username) => void save({ ...form, username })}
            />
          }
        />
        <SettingItem
          layout="stack"
          title="Password"
          description={
            settings.has_password
              ? "Leave empty to keep the current password."
              : "Set a password before enabling LAN access."
          }
          control={
            <FieldText
              aria-label="Basic-auth password"
              placeholder={
                settings.has_password ? "unchanged" : "Set a password"
              }
              type="password"
              value={form.password}
              onChange={(password) => changeForm({ password })}
              onCommit={(password) => void save({ ...form, password })}
            />
          }
        />
        {guardError ? <FieldError>{guardError}</FieldError> : null}
        {saveError ? <FieldError>{saveError}</FieldError> : null}
        {form.lan_enabled && reachableUrls.length > 0 ? (
          <div className={styles.reachable}>
            <div className={styles.urlList}>
              <strong>Reachable URLs</strong>
              {reachableUrls.map((url) => (
                <a href={url} key={url} rel="noreferrer" target="_blank">
                  {url}
                </a>
              ))}
            </div>
            {qrUrl ? <QrCode url={qrUrl} /> : null}
          </div>
        ) : null}
      </Surface>
    </SettingsGroup>
  );
}
