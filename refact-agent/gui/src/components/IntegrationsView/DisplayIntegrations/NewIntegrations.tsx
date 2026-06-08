import { FC } from "react";
import {
  IntegrationWithIconRecord,
  NotConfiguredIntegrationWithIconRecord,
} from "../../../services/refact";
import { IntegrationCard } from "./IntegrationCard";
import styles from "./DisplayIntegrations.module.css";

type NewIntegrationsProps = {
  availableIntegrationsToConfigure?: NotConfiguredIntegrationWithIconRecord[];
  handleIntegrationShowUp: (
    integration:
      | IntegrationWithIconRecord
      | NotConfiguredIntegrationWithIconRecord,
  ) => void;
};

export const NewIntegrations: FC<NewIntegrationsProps> = ({
  availableIntegrationsToConfigure,
  handleIntegrationShowUp,
}) => (
  <div className={styles.grid}>
    {availableIntegrationsToConfigure?.map((integration, index) => (
      <IntegrationCard
        isNotConfigured
        key={`project-${index}-${JSON.stringify(integration.integr_config_path)}`}
        integration={integration}
        handleIntegrationShowUp={handleIntegrationShowUp}
      />
    ))}
  </div>
);
