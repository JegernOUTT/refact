import React from "react";
import { ArrowLeft } from "lucide-react";
import { useAppDispatch } from "../../hooks";
import { change } from "../Pages/pagesSlice";
import type { Page } from "../Pages/pagesSlice";
import type { Config } from "../Config/configSlice";
import { SETTINGS_SECTIONS, settingsPageToSection, settingsSectionToPage } from "./index";
import type { SettingsSectionId } from "./settingsSections";
import { Button, SettingsShell, Surface } from "../../components/ui";
import { Providers } from "../Providers";
import { DefaultModels } from "../DefaultModels";
import { Customization } from "../Customization";
import { Integrations } from "../Integrations";
import { SchedulerPanel } from "../Scheduler";
import { GeneralSettingsSection } from "./GeneralSettingsSection";

import styles from "./SettingsHub.module.css";

export interface SettingsHubProps {
  page: Page;
  onBack: () => void;
  host: Config["host"];
  tabbed: Config["tabbed"];
}

export const SettingsHub: React.FC<SettingsHubProps> = ({
  page,
  onBack,
  host,
  tabbed,
}) => {
  const dispatch = useAppDispatch();
  const activeSection: SettingsSectionId = settingsPageToSection(page) ?? "general";

  const handleSectionChange = (id: string) => {
    dispatch(change(settingsSectionToPage(id as SettingsSectionId)));
  };

  const renderContent = () => {
    switch (activeSection) {
      case "general":
        return <GeneralSettingsSection />;
      case "providers":
        return (
          <Providers
            embedded
            host={host}
            tabbed={tabbed}
            backFromProviders={onBack}
          />
        );
      case "models":
        return (
          <DefaultModels
            embedded
            host={host}
            tabbed={tabbed}
            backFromDefaultModels={onBack}
            draftId={page.name === "default models" ? page.draftId : undefined}
          />
        );
      case "customization":
        return (
          <Customization
            embedded
            host={host}
            tabbed={tabbed}
            backFromCustomization={onBack}
            initialKind={page.name === "customization" ? page.kind : undefined}
            initialConfigId={page.name === "customization" ? page.configId : undefined}
            draftId={page.name === "customization" ? page.draftId : undefined}
          />
        );
      case "integrations":
        return (
          <Integrations
            embedded
            host={host}
            tabbed={tabbed}
            backFromIntegrations={onBack}
            onCloseIntegrations={onBack}
            handlePaddingShift={(_state: boolean) => undefined}
          />
        );
      case "scheduler":
        return <SchedulerPanel embedded onBack={onBack} />;
      case "documentation":
        return (
          <Surface className={styles.placeholder} variant="surface-1">
            <p className={styles.placeholderText}>
              Documentation settings — coming up
            </p>
          </Surface>
        );
      case "extensions":
        return (
          <Surface className={styles.placeholder} variant="surface-1">
            <p className={styles.placeholderText}>
              Extensions settings — coming up
            </p>
          </Surface>
        );
    }
  };

  return (
    <div className={styles.hub}>
      <div className={styles.header}>
        <Button variant="ghost" leftIcon={ArrowLeft} onClick={onBack}>
          Back
        </Button>
      </div>
      <SettingsShell
        className={styles.shell}
        sections={SETTINGS_SECTIONS}
        active={activeSection}
        onSectionChange={handleSectionChange}
        title="Settings"
        description="Configure your AI coding assistant"
      >
        <div className={styles.content}>{renderContent()}</div>
      </SettingsShell>
    </div>
  );
};
