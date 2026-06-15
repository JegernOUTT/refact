import { TriangleAlert } from "lucide-react";
import { FC } from "react";
import { Integration } from "../../../services/refact";
import { selectConfig } from "../../../features/Config/configSlice";
import { useAppSelector, useEventsBusForIDE } from "../../../hooks";
import { DeletePopover } from "../../DeletePopover";
import { Badge, Button, Flex, Surface, Text } from "../../ui";
import styles from "./IntegrationForm.module.css";

type ErrorStateProps = {
  integration: Integration;
  onDelete: (path: string) => void;
  isApplying: boolean;
  isDeletingIntegration: boolean;
};

export const ErrorState: FC<ErrorStateProps> = ({
  onDelete,
  isApplying,
  isDeletingIntegration,
  integration,
}) => {
  const config = useAppSelector(selectConfig);
  const { openFile } = useEventsBusForIDE();

  const { integr_name } = integration;
  const { error_msg, integr_config_path, error_line } =
    integration.error_log[0];

  return (
    <Surface
      animated="rise"
      className={styles.errorSurface}
      radius="card"
      variant="glass"
    >
      <Flex direction="column" align="start" gap="4">
        <Text as="p" size="2" color="gray">
          Whoops, this integration has a syntax error in the config file. You
          can fix this by editing the config file.
        </Text>
        <Badge tone="danger">
          <TriangleAlert size={14} /> {error_msg}
        </Badge>
        <Flex
          align="center"
          className={styles.errorActions}
          gap="2"
          wrap="wrap"
        >
          {config.host !== "web" && (
            <Button
              variant="soft"
              title={`Open ${integr_name}.yaml configuration file in your IDE`}
              onClick={() =>
                openFile({
                  file_path: integr_config_path,
                  line: error_line === 0 ? 1 : error_line,
                })
              }
            >
              Open {integr_name}.yaml
            </Button>
          )}
          <DeletePopover
            itemName={integr_name}
            deleteBy={integr_config_path}
            isDisabled={isApplying}
            isDeleting={isDeletingIntegration}
            handleDelete={onDelete}
          />
        </Flex>
      </Flex>
    </Surface>
  );
};
