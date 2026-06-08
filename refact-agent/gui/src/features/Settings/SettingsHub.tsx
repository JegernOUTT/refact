import React from "react";
import { ArrowLeft } from "lucide-react";
import { useAppDispatch } from "../../hooks";
import { change } from "../Pages/pagesSlice";
import type { Page } from "../Pages/pagesSlice";
import type { Config } from "../Config/configSlice";
import { SETTINGS_SECTIONS } from "./settingsSections";
import type { SettingsSectionId } from "./settingsSections";
import { settingsPageToSection, settingsSectionToPage } from "./settingsRoutes";
import { Button, SettingsShell } from "../../components/ui";
import { Providers } from "../Providers";
import { DefaultModels } from "../DefaultModels";
import { Customization } from "../Customization";
import { Integrations } from "../Integrations";
import { SchedulerPanel } from "../Scheduler";
import { Extensions } from "../Extensions";
import { GeneralSettingsSection } from "./GeneralSettingsSection";
import { DocumentationSettingsSection } from "./DocumentationSettingsSection";

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
  const activeSection: SettingsSectionId =
    settingsPageToSection(page) ?? "general";

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
      case "models": {
        const draftId =
          page.name === "default models" ? page.draftId : undefined;

        return (
          <DefaultModels
            key={`models-${draftId ?? ""}`}
            embedded
            host={host}
            tabbed={tabbed}
            backFromDefaultModels={onBack}
            draftId={draftId}
          />
        );
      }
      case "customization": {
        const kind = page.name === "customization" ? page.kind : undefined;
        const configId =
          page.name === "customization" ? page.configId : undefined;
        const draftId =
          page.name === "customization" ? page.draftId : undefined;

        return (
          <Customization
            key={`cust-${kind ?? ""}-${configId ?? ""}-${draftId ?? ""}`}
            embedded
            host={host}
            tabbed={tabbed}
            backFromCustomization={onBack}
            initialKind={kind}
            initialConfigId={configId}
            draftId={draftId}
          />
        );
      }
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
        return <DocumentationSettingsSection />;
      case "extensions": {
        const tab = page.name === "extensions" ? page.tab : undefined;
        const itemId = page.name === "extensions" ? page.itemId : undefined;
        const draftId = page.name === "extensions" ? page.draftId : undefined;

        return (
          <Extensions
            key={`ext-${tab ?? ""}-${itemId ?? ""}-${draftId ?? ""}`}
            embedded
            host={host}
            tabbed={tabbed}
            backFromExtensions={onBack}
            initialTab={tab}
            initialItemId={itemId}
            draftId={draftId}
          />
        );
      }
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
