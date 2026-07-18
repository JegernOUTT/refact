import { useState } from "react";
import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";
import { ExternalLink, Power } from "lucide-react";

import {
  Button,
  FieldError,
  FieldText,
  SettingItem,
  Surface,
} from "../../../components/ui";
import { useShutdownDaemonMutation } from "../../../services/refact/daemon";
import { SettingsGroup } from "../../Settings/SettingsSection";
import styles from "./SettingsPage.module.css";

const SHUTDOWN_CONFIRMATION = "shutdown";

function errorMessage(error: unknown): string {
  const queryError = error as FetchBaseQueryError | undefined;
  if (queryError && "data" in queryError) {
    const data = queryError.data;
    if (typeof data === "object" && data !== null && "error" in data) {
      const message = (data as { error?: unknown }).error;
      if (typeof message === "string" && message) return message;
    }
  }
  return "Could not shutdown the daemon.";
}

export function DangerSection() {
  const [confirmation, setConfirmation] = useState("");
  const [notice, setNotice] = useState<string | null>(null);
  const [shutdownError, setShutdownError] = useState<string | null>(null);
  const [shutdown, shutdownState] = useShutdownDaemonMutation();

  async function runShutdown() {
    setShutdownError(null);
    setNotice(null);
    try {
      await shutdown({ reason: "dashboard_settings_shutdown" }).unwrap();
      setNotice("Shutdown requested. This dashboard will become unavailable.");
    } catch (error) {
      setShutdownError(errorMessage(error));
    }
  }

  return (
    <SettingsGroup title="Danger zone">
      <Surface className={styles.dangerSurface} variant="glass">
        <SettingItem
          title="Legacy project picker"
          description="Open the server-rendered picker for recovery and compatibility."
          control={
            <Button asChild leftIcon={ExternalLink} size="sm" variant="soft">
              <a href="/picker">Open legacy picker</a>
            </Button>
          }
        />
        <SettingItem
          layout="stack"
          title="Shutdown daemon"
          description='Type "shutdown" to enable the stop action.'
          control={
            <div className={styles.shutdownControl}>
              <FieldText
                aria-label="Shutdown confirmation"
                autoComplete="off"
                value={confirmation}
                onChange={setConfirmation}
              />
              <Button
                disabled={confirmation !== SHUTDOWN_CONFIRMATION}
                leftIcon={Power}
                loading={shutdownState.isLoading}
                onClick={() => void runShutdown()}
                size="sm"
                variant="danger"
              >
                Shutdown daemon
              </Button>
            </div>
          }
        />
        {shutdownError ? <FieldError>{shutdownError}</FieldError> : null}
        {notice ? <p className={styles.notice}>{notice}</p> : null}
      </Surface>
    </SettingsGroup>
  );
}
