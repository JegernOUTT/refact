import { FC } from "react";
import {
  IntegrationWithIconRecord,
  NotConfiguredIntegrationWithIconRecord,
} from "../../../services/refact";
import { SettingsGroup } from "../../../features/Settings/SettingsSection";
import { IntegrationCard } from "./IntegrationCard";
import styles from "./DisplayIntegrations.module.css";

type GlobalIntegrationsProps = {
  globalIntegrations?: IntegrationWithIconRecord[];
  handleIntegrationShowUp: (
    integration:
      | IntegrationWithIconRecord
      | NotConfiguredIntegrationWithIconRecord,
  ) => void;
};

export const GlobalIntegrations: FC<GlobalIntegrationsProps> = ({
  globalIntegrations,
  handleIntegrationShowUp,
}) => {
  const count = globalIntegrations?.length ?? 0;

  return (
    <SettingsGroup
      title={`Globally configured · ${count} ${
        count !== 1 ? "integrations" : "integration"
      }`}
    >
      <p className={styles.groupDescription}>
        Global configurations are shared in your IDE and available for all
        projects.
      </p>
      {globalIntegrations ? (
        <div className={styles.cards}>
          {globalIntegrations.map((integration, index) => (
            <IntegrationCard
              key={`${index}-${integration.integr_config_path}`}
              integration={integration}
              handleIntegrationShowUp={handleIntegrationShowUp}
            />
          ))}
        </div>
      ) : null}
    </SettingsGroup>
  );
};
