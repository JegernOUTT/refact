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
  <section className={styles.section}>
    <h4 className={styles.sectionTitle}>Add new integration</h4>
    <div className={styles.grid}>
      {availableIntegrationsToConfigure &&
        Object.entries(availableIntegrationsToConfigure).map(
          ([_projectPath, integration], index) => (
            <IntegrationCard
              isNotConfigured
              key={`project-${index}-${JSON.stringify(
                integration.integr_config_path,
              )}`}
              integration={integration}
              handleIntegrationShowUp={handleIntegrationShowUp}
            />
          ),
        )}
    </div>
  </section>
);
