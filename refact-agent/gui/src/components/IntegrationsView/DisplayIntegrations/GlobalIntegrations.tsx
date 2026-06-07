import { Settings } from "lucide-react";
import { FC } from "react";
import {
  IntegrationWithIconRecord,
  NotConfiguredIntegrationWithIconRecord,
} from "../../../services/refact";
import { Icon } from "../../ui";
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
  return (
    <section className={styles.section}>
      <h4 className={styles.sectionTitle}>
        <Icon icon={Settings} size="md" tone="muted" />
        Globally configured {globalIntegrations?.length ?? 0}{" "}
        {(globalIntegrations?.length ?? 0) !== 1
          ? "integrations"
          : "integration"}
      </h4>
      <p className={styles.muted}>
        Global configurations are shared in your IDE and available for all your
        projects.
      </p>
      {globalIntegrations && (
        <div className={styles.cards}>
          {globalIntegrations.map((integration, index) => (
            <IntegrationCard
              key={`${index}-${integration.integr_config_path}`}
              integration={integration}
              handleIntegrationShowUp={handleIntegrationShowUp}
            />
          ))}
        </div>
      )}
    </section>
  );
};
