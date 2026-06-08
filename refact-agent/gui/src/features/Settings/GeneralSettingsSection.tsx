import React, { useCallback } from "react";
import { useAppDispatch, useAppSelector, useEventsBusForIDE } from "../../hooks";
import { useAppearance } from "../../hooks/useAppearance";
import {
  changeFeature,
  selectConfig,
  selectFeatures,
  selectThemeMode,
  setThemeMode,
} from "../Config/configSlice";
import { Button, FieldSwitch, Select, SettingItem } from "../../components/ui";
import styles from "./GeneralSettingsSection.module.css";

export const GeneralSettingsSection: React.FC = () => {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const themeMode = useAppSelector(selectThemeMode);
  const features = useAppSelector(selectFeatures);
  const { openSettings } = useEventsBusForIDE();
  const { appearance } = useAppearance();

  const handleAppearanceChange = useCallback(
    (value: string) => {
      dispatch(setThemeMode(value as "light" | "dark" | "inherit"));
    },
    [dispatch],
  );

  const handleFeatureToggle = useCallback(
    (feature: string, value: boolean) => {
      dispatch(changeFeature({ feature, value }));
    },
    [dispatch],
  );

  const hostLabel =
    config.host === "vscode"
      ? "Extension Settings"
      : config.host === "jetbrains"
        ? "Plugin Settings"
        : null;

  const featureEntries = Object.entries(features ?? {});
  const lockedFeatures = new Set(["ast", "vecdb"]);

  return (
    <div className={`${styles.section} rf-enter`}>
      <div className={`${styles.group} rf-stagger`}>
        <h2 className={styles.groupTitle}>Appearance</h2>
        <SettingItem
          className="rf-enter"
          title="Theme"
          description="Choose light, dark, or inherit from the host environment."
          control={
            <Select value={themeMode ?? appearance} onValueChange={handleAppearanceChange}>
              <Select.Trigger className={styles.select} />
              <Select.Content>
                <Select.Item value="dark">Dark</Select.Item>
                <Select.Item value="light">Light</Select.Item>
                <Select.Item value="inherit">Inherit</Select.Item>
              </Select.Content>
            </Select>
          }
        />
      </div>

      {featureEntries.length > 0 && (
        <div className={`${styles.group} rf-stagger`}>
          <h2 className={styles.groupTitle}>Feature Flags</h2>
          {featureEntries.map(([feature, value]) => {
            const locked = lockedFeatures.has(feature);
            return (
              <SettingItem
                key={feature}
                className="rf-enter"
                title={feature}
                description={locked ? "Managed in settings" : undefined}
                control={
                  <FieldSwitch
                    checked={!!value}
                    onChange={() => handleFeatureToggle(feature, !value)}
                    disabled={locked}
                  />
                }
              />
            );
          })}
        </div>
      )}

      {(hostLabel ?? config.currentWorkspaceName) && (
        <div className={`${styles.group} rf-stagger`}>
          <h2 className={styles.groupTitle}>Runtime Info</h2>
          {config.currentWorkspaceName && (
            <SettingItem
              className="rf-enter"
              title="Workspace"
              description={config.currentWorkspaceName}
            />
          )}
          {hostLabel && (
            <SettingItem
              className="rf-enter"
              title="Host"
              description={config.host}
              control={
                <Button variant="soft" onClick={openSettings}>
                  {hostLabel}
                </Button>
              }
            />
          )}
        </div>
      )}
    </div>
  );
};
