import classNames from "classnames";
import { Settings } from "lucide-react";
import { FC } from "react";
import {
  IntegrationWithIconRecord,
  NotConfiguredIntegrationWithIconRecord,
} from "../../../services/refact";
import { formatPathName } from "../../../utils/formatPathName";
import { Markdown } from "../../Markdown";
import { Icon } from "../../ui";
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
      const formattedProjectName = formatPathName(
        projectPath,
        "```.../",
        "/```",
      );

      return (
        <section className={styles.section} key={`project-group-${index}`}>
          <h4 className={classNames(styles.sectionTitle, styles.sectionTitleWrap)}>
            <Icon icon={Settings} size="md" tone="muted" />
            In
            <Markdown>{formattedProjectName}</Markdown>
            configured {integrations.length}{" "}
            {integrations.length !== 1 ? "integrations" : "integration"}
          </h4>
          <p className={styles.muted}>
            Folder-specific integrations are local integrations, which are
            shared only in folder-specific scope.
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
        </section>
      );
    },
  );
};
