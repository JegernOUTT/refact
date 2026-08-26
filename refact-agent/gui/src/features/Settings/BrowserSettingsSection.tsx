import React, { useEffect, useMemo, useState } from "react";

import {
  Button,
  FieldSwitch,
  FieldText,
  SettingItem,
} from "../../components/ui";
import { TextArea } from "../../components/TextArea";
import {
  useGetBrowserSettingsQuery,
  useSaveBrowserSettingsMutation,
} from "../../services/refact/browserSettings";
import type {
  BrowserCaptureSettings,
  BrowserLaunchSettings,
  BrowserLifecycleSettings,
  BrowserSettings,
  BrowserTimingSettings,
  BrowserViewportGroup,
  BrowserViewportSettings,
} from "../../services/refact/browserSettings";
import { SettingsGroup, SettingsSection } from "./SettingsSection";
import styles from "./BrowserSettingsSection.module.css";

function errorMessage(error: unknown): string {
  if (!error || typeof error !== "object") return "The request failed.";
  if ("error" in error && typeof error.error === "string") return error.error;
  if ("data" in error) {
    const data = error.data;
    if (typeof data === "string") return data;
    if (data && typeof data === "object") {
      if ("message" in data && typeof data.message === "string") {
        return data.message;
      }
      if ("detail" in data && typeof data.detail === "string") {
        return data.detail;
      }
    }
  }
  return "The request failed.";
}

function parseNumber(value: string, integer = true): number {
  const parsed = integer
    ? Number.parseInt(value, 10)
    : Number.parseFloat(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function parseExtraArgs(value: string): string[] {
  return value
    .split(/[\n,]/)
    .map((argument) => argument.trim())
    .filter(Boolean);
}

interface NumberSettingProps {
  label: string;
  ariaLabel?: string;
  value: number;
  onChange: (value: number) => void;
  integer?: boolean;
}

function NumberSetting({
  ariaLabel,
  integer = true,
  label,
  onChange,
  value,
}: NumberSettingProps) {
  return (
    <SettingItem
      className="rf-enter"
      title={label}
      control={
        <FieldText
          className={styles.control}
          aria-label={ariaLabel ?? label}
          type="number"
          step={integer ? 1 : "any"}
          value={value.toString()}
          onChange={(nextValue) => onChange(parseNumber(nextValue, integer))}
        />
      }
    />
  );
}

interface ViewportSettingsProps {
  label: string;
  value: BrowserViewportSettings;
  onChange: (value: BrowserViewportSettings) => void;
}

function ViewportSettings({ label, onChange, value }: ViewportSettingsProps) {
  return (
    <SettingsGroup title={`Viewport — ${label}`}>
      <NumberSetting
        label="Width"
        ariaLabel={`${label} width`}
        value={value.width}
        onChange={(width) => onChange({ ...value, width })}
      />
      <NumberSetting
        label="Height"
        ariaLabel={`${label} height`}
        value={value.height}
        onChange={(height) => onChange({ ...value, height })}
      />
      <NumberSetting
        integer={false}
        label="Scale factor"
        ariaLabel={`${label} scale factor`}
        value={value.scale_factor}
        onChange={(scale_factor) => onChange({ ...value, scale_factor })}
      />
    </SettingsGroup>
  );
}

export const BrowserSettingsSection: React.FC = () => {
  const { data, error, isFetching } = useGetBrowserSettingsQuery(undefined);
  const [saveSettings, saveState] = useSaveBrowserSettingsMutation();
  const [draft, setDraft] = useState<BrowserSettings | null>(data ?? null);
  const [extraArgsText, setExtraArgsText] = useState(
    data?.launch.extra_args.join("\n") ?? "",
  );
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (data) {
      setDraft(data);
      setExtraArgsText(data.launch.extra_args.join("\n"));
      setSaveError(null);
    }
  }, [data]);

  const settingsToSave = useMemo<BrowserSettings | null>(
    () =>
      draft
        ? {
            ...draft,
            launch: {
              ...draft.launch,
              extra_args: parseExtraArgs(extraArgsText),
            },
          }
        : null,
    [draft, extraArgsText],
  );
  const unchanged = useMemo(
    () =>
      settingsToSave === null ||
      data === undefined ||
      JSON.stringify(settingsToSave) === JSON.stringify(data),
    [data, settingsToSave],
  );

  const updateLaunch = <Key extends keyof BrowserLaunchSettings>(
    key: Key,
    value: BrowserLaunchSettings[Key],
  ) => {
    setDraft((current) =>
      current
        ? { ...current, launch: { ...current.launch, [key]: value } }
        : current,
    );
    setSaveError(null);
  };

  const updateLifecycle = <Key extends keyof BrowserLifecycleSettings>(
    key: Key,
    value: BrowserLifecycleSettings[Key],
  ) => {
    setDraft((current) =>
      current
        ? { ...current, lifecycle: { ...current.lifecycle, [key]: value } }
        : current,
    );
    setSaveError(null);
  };

  const updateViewport = (
    key: keyof BrowserViewportGroup,
    value: BrowserViewportSettings,
  ) => {
    setDraft((current) =>
      current
        ? { ...current, viewport: { ...current.viewport, [key]: value } }
        : current,
    );
    setSaveError(null);
  };

  const updateTiming = <Key extends keyof BrowserTimingSettings>(
    key: Key,
    value: BrowserTimingSettings[Key],
  ) => {
    setDraft((current) =>
      current
        ? { ...current, timing: { ...current.timing, [key]: value } }
        : current,
    );
    setSaveError(null);
  };

  const updateCapture = <Key extends keyof BrowserCaptureSettings>(
    key: Key,
    value: BrowserCaptureSettings[Key],
  ) => {
    setDraft((current) =>
      current
        ? { ...current, capture: { ...current.capture, [key]: value } }
        : current,
    );
    setSaveError(null);
  };

  const handleSave = async () => {
    if (!settingsToSave) return;
    setSaveError(null);
    try {
      const saved = await saveSettings(settingsToSave).unwrap();
      setDraft(saved);
      setExtraArgsText(saved.launch.extra_args.join("\n"));
    } catch (requestError) {
      setSaveError(errorMessage(requestError));
    }
  };

  if (isFetching && !draft) {
    return <p className={styles.status}>Loading browser settings…</p>;
  }

  if (!draft) {
    return (
      <p className={styles.error} role="alert">
        Could not load browser settings: {errorMessage(error)}
      </p>
    );
  }

  return (
    <SettingsSection
      title="Browser"
      description="Configure browser launch, lifecycle, viewport, timing, and capture defaults."
      width="wide"
      actions={
        <Button
          variant="primary"
          loading={saveState.isLoading}
          disabled={saveState.isLoading || unchanged}
          onClick={() => void handleSave()}
        >
          Save
        </Button>
      }
    >
      <SettingsGroup title="Launch">
        <SettingItem title="Chrome path">
          <FieldText
            className={styles.control}
            aria-label="Chrome path"
            value={draft.launch.chrome_path}
            onChange={(value) => updateLaunch("chrome_path", value)}
          />
        </SettingItem>
        <SettingItem title="Headless">
          <FieldSwitch
            aria-label="Headless"
            checked={draft.launch.headless}
            onChange={(value) => updateLaunch("headless", value)}
          />
        </SettingItem>
        <SettingItem title="Chromium sandbox">
          <FieldSwitch
            aria-label="Chromium sandbox"
            checked={draft.launch.chromium_sandbox}
            onChange={(value) => updateLaunch("chromium_sandbox", value)}
          />
        </SettingItem>
        <SettingItem title="Ignore HTTPS errors">
          <FieldSwitch
            aria-label="Ignore HTTPS errors"
            checked={draft.launch.ignore_https_errors}
            onChange={(value) => updateLaunch("ignore_https_errors", value)}
          />
        </SettingItem>
        <SettingItem title="Mask passwords">
          <FieldSwitch
            aria-label="Mask passwords"
            checked={draft.launch.mask_passwords}
            onChange={(value) => updateLaunch("mask_passwords", value)}
          />
        </SettingItem>
        <SettingItem
          title="Extra arguments"
          description="Separate arguments with commas or new lines."
        >
          <TextArea
            className={styles.control}
            aria-label="Extra arguments"
            rows={3}
            value={extraArgsText}
            onChange={(event) => {
              setExtraArgsText(event.target.value);
              setSaveError(null);
            }}
          />
        </SettingItem>
        <SettingItem title="Downloads directory">
          <FieldText
            className={styles.control}
            aria-label="Downloads directory"
            value={draft.launch.downloads_dir}
            onChange={(value) => updateLaunch("downloads_dir", value)}
          />
        </SettingItem>
        <SettingItem title="Proxy server">
          <FieldText
            className={styles.control}
            aria-label="Proxy server"
            value={draft.launch.proxy_server}
            onChange={(value) => updateLaunch("proxy_server", value)}
          />
        </SettingItem>
        <SettingItem title="Proxy bypass">
          <FieldText
            className={styles.control}
            aria-label="Proxy bypass"
            value={draft.launch.proxy_bypass}
            onChange={(value) => updateLaunch("proxy_bypass", value)}
          />
        </SettingItem>
      </SettingsGroup>

      <SettingsGroup title="Lifecycle">
        <NumberSetting
          label="Idle timeout seconds"
          value={draft.lifecycle.idle_timeout_secs}
          onChange={(value) => updateLifecycle("idle_timeout_secs", value)}
        />
        <SettingItem title="Evict idle attached browsers">
          <FieldSwitch
            aria-label="Evict idle attached browsers"
            checked={draft.lifecycle.evict_idle_attached}
            onChange={(value) => updateLifecycle("evict_idle_attached", value)}
          />
        </SettingItem>
        <NumberSetting
          label="Monitor interval seconds"
          value={draft.lifecycle.monitor_interval_secs}
          onChange={(value) => updateLifecycle("monitor_interval_secs", value)}
        />
        <NumberSetting
          label="Relaunch settle milliseconds"
          value={draft.lifecycle.relaunch_settle_ms}
          onChange={(value) => updateLifecycle("relaunch_settle_ms", value)}
        />
      </SettingsGroup>

      <ViewportSettings
        label="Desktop"
        value={draft.viewport.desktop}
        onChange={(value) => updateViewport("desktop", value)}
      />
      <ViewportSettings
        label="Mobile"
        value={draft.viewport.mobile}
        onChange={(value) => updateViewport("mobile", value)}
      />
      <ViewportSettings
        label="Tablet"
        value={draft.viewport.tablet}
        onChange={(value) => updateViewport("tablet", value)}
      />

      <SettingsGroup title="Timing">
        <NumberSetting
          label="Default wait timeout milliseconds"
          value={draft.timing.default_wait_timeout_ms}
          onChange={(value) => updateTiming("default_wait_timeout_ms", value)}
        />
        <NumberSetting
          label="Maximum wait timeout milliseconds"
          value={draft.timing.max_wait_timeout_ms}
          onChange={(value) => updateTiming("max_wait_timeout_ms", value)}
        />
        <NumberSetting
          label="Default poll interval milliseconds"
          value={draft.timing.default_poll_interval_ms}
          onChange={(value) => updateTiming("default_poll_interval_ms", value)}
        />
      </SettingsGroup>

      <SettingsGroup title="Capture">
        <NumberSetting
          label="Default ARIA snapshot characters"
          value={draft.capture.default_aria_snapshot_chars}
          onChange={(value) =>
            updateCapture("default_aria_snapshot_chars", value)
          }
        />
        <NumberSetting
          label="Maximum ARIA snapshot characters"
          value={draft.capture.max_aria_snapshot_chars}
          onChange={(value) => updateCapture("max_aria_snapshot_chars", value)}
        />
        <NumberSetting
          label="Maximum DOM snapshot characters"
          value={draft.capture.max_dom_snapshot_chars}
          onChange={(value) => updateCapture("max_dom_snapshot_chars", value)}
        />
        <NumberSetting
          label="Maximum inline snapshot bytes"
          value={draft.capture.max_inline_snapshot_bytes}
          onChange={(value) =>
            updateCapture("max_inline_snapshot_bytes", value)
          }
        />
        <NumberSetting
          label="Maximum extracted links"
          value={draft.capture.max_extract_links}
          onChange={(value) => updateCapture("max_extract_links", value)}
        />
        <NumberSetting
          label="Maximum extracted table rows"
          value={draft.capture.max_extract_table_rows}
          onChange={(value) => updateCapture("max_extract_table_rows", value)}
        />
        <NumberSetting
          label="Default all texts"
          value={draft.capture.default_all_texts}
          onChange={(value) => updateCapture("default_all_texts", value)}
        />
        {saveError ? (
          <p className={styles.error} role="alert">
            Could not save browser settings: {saveError}
          </p>
        ) : null}
      </SettingsGroup>
    </SettingsSection>
  );
};
