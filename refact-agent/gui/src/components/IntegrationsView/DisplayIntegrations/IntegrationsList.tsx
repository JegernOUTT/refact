import { FC } from "react";
import { Store } from "lucide-react";
import {
  IntegrationWithIconRecord,
  NotConfiguredIntegrationWithIconRecord,
} from "../../../services/refact";
import {
  SettingsGroup,
  SettingsSection,
} from "../../../features/Settings/SettingsSection";
import { GlobalIntegrations } from "./GlobalIntegrations";
import { NewIntegrations } from "./NewIntegrations";
import { ProjectIntegrations } from "./ProjectIntegrations";
import { useAppDispatch } from "../../../hooks";
import { push } from "../../../features/Pages/pagesSlice";
import { Button, Flex } from "../../ui";
import { MCPImportDialog } from "../MCPImportDialog";
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
    <SettingsSection
      title="Integrations"
      description="Connect Refact.ai Agent to command-line tools, MCP servers, and workspace services."
      actions={
        <Flex align="center" gap="2" wrap="wrap">
          <MCPImportDialog />
          <Button
            variant="soft"
            size="sm"
            rightIcon={Store}
            onClick={() => dispatch(push({ name: "mcp marketplace" }))}
          >
            Browse MCP Marketplace
          </Button>
        </Flex>
      }
      width="wide"
      className={styles.settingsSection}
    >
      <GlobalIntegrations
        globalIntegrations={globalIntegrations}
        handleIntegrationShowUp={handleIntegrationShowUp}
      />
      <ProjectIntegrations
        groupedProjectIntegrations={groupedProjectIntegrations}
        handleIntegrationShowUp={handleIntegrationShowUp}
      />
      <SettingsGroup title="Add new integration">
        <p className={styles.groupDescription}>
          Configure another integration or MCP command-line server from the
          available templates.
        </p>
        <NewIntegrations
          availableIntegrationsToConfigure={availableIntegrationsToConfigure}
          handleIntegrationShowUp={handleIntegrationShowUp}
        />
      </SettingsGroup>
    </SettingsSection>
  );
};
