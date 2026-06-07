import { FC } from "react";
import {
  IntegrationWithIconRecord,
  NotConfiguredIntegrationWithIconRecord,
} from "../../../services/refact";
import { GlobalIntegrations } from "./GlobalIntegrations";
import { NewIntegrations } from "./NewIntegrations";
import { ProjectIntegrations } from "./ProjectIntegrations";
import { useAppDispatch } from "../../../hooks";
import { push } from "../../../features/Pages/pagesSlice";
import { Button } from "../../ui";
import styles from "./DisplayIntegrations.module.css";

type IntegrationsListProps = {
  globalIntegrations?: IntegrationWithIconRecord[];
  groupedProjectIntegrations?: Record<string, IntegrationWithIconRecord[]>;
  availableIntegrationsToConfigure?: NotConfiguredIntegrationWithIconRecord[];
  handleIntegrationShowUp: (
    integration:
      | IntegrationWithIconRecord
      | NotConfiguredIntegrationWithIconRecord,
  ) => void;
};

export const IntegrationsList: FC<IntegrationsListProps> = ({
  globalIntegrations,
  groupedProjectIntegrations,
  availableIntegrationsToConfigure,
  handleIntegrationShowUp,
}) => {
  const dispatch = useAppDispatch();

  return (
    <div className={styles.list}>
      <div className={styles.introRow}>
        <p className={styles.description}>
          Integrations allow Refact.ai Agent to interact with other services and
          tools
        </p>
        <Button
          variant="soft"
          size="sm"
          onClick={() => dispatch(push({ name: "mcp marketplace" }))}
        >
          Browse MCP Marketplace
        </Button>
      </div>
      <GlobalIntegrations
        globalIntegrations={globalIntegrations}
        handleIntegrationShowUp={handleIntegrationShowUp}
      />
      <ProjectIntegrations
        groupedProjectIntegrations={groupedProjectIntegrations}
        handleIntegrationShowUp={handleIntegrationShowUp}
      />
      <NewIntegrations
        availableIntegrationsToConfigure={availableIntegrationsToConfigure}
        handleIntegrationShowUp={handleIntegrationShowUp}
      />
    </div>
  );
};
