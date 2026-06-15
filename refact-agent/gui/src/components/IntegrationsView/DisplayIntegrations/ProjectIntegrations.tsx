import { FC } from "react";
import {
  IntegrationWithIconRecord,
  NotConfiguredIntegrationWithIconRecord,
} from "../../../services/refact";
import { SettingsGroup } from "../../../features/Settings/SettingsSection";
import { formatPathName } from "../../../utils/formatPathName";
import { IntegrationCard } from "./IntegrationCard";
import styles from "./DisplayIntegrations.module.css";

type ProjectIntegrationsProps = {
  groupedProjectIntegrations?: Record<string, IntegrationWithIconRecord[]>;
  handleIntegrationShowUp: (
    integration:
      | IntegrationWithIconRecord
      | NotConfiguredIntegrationWithIconRecord,
  ) => void;
};

export const ProjectIntegrations: FC<ProjectIntegrationsProps> = ({
  groupedProjectIntegrations,
  handleIntegrationShowUp,
}) => {
  if (!groupedProjectIntegrations) return null;

  return Object.entries(groupedProjectIntegrations).map(
    ([projectPath, integrations], index) => {
      const formattedProjectName = formatPathName(projectPath, ".../");

      return (
        <SettingsGroup
          title={`In ${formattedProjectName} · ${integrations.length} ${
            integrations.length !== 1 ? "integrations" : "integration"
          }`}
          key={`project-group-${index}`}
        >
          <p className={styles.groupDescription}>
            Folder-specific integrations are shared only in this project scope.
          </p>
          <div className={styles.cards}>
            {integrations.map((integration, subIndex) => (
              <IntegrationCard
                key={`project-${index}-${subIndex}-${integration.integr_config_path}`}
                integration={integration}
                handleIntegrationShowUp={handleIntegrationShowUp}
              />
            ))}
          </div>
        </SettingsGroup>
      );
    },
  );
};
