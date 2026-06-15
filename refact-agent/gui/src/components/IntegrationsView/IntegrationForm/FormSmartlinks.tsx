import { FC } from "react";
import { Integration, SmartLink as TSmartLink } from "../../../services/refact";
import { selectConfig } from "../../../features/Config/configSlice";
import { useAppSelector, useEventsBusForIDE } from "../../../hooks";
import { Button, Flex } from "../../ui";
import { SmartLink } from "../../SmartLink";
import styles from "./IntegrationForm.module.css";

type FormSmartlinksProps = {
  integration: Integration;
  smartlinks: TSmartLink[] | undefined;
};

export const FormSmartlinks: FC<FormSmartlinksProps> = ({
  smartlinks,
  integration,
}) => {
  const config = useAppSelector(selectConfig);
  const { openFile } = useEventsBusForIDE();

  const { integr_name, project_path, integr_config_path, integr_values } =
    integration;

  const available = integr_values
    ? (integr_values.available as Record<string, boolean>)
    : {};

  if (!smartlinks?.length) return null;

  return (
    <Flex className={styles.smartlinks} direction="column" gap="1">
      <Flex
        align="start"
        className={styles.smartlinksRow}
        direction="row"
        justify="between"
        gap="4"
        wrap="wrap"
      >
        <Flex
          align="center"
          className={styles.smartlinksActions}
          gap="3"
          justify="start"
          wrap="wrap"
        >
          <h6 className={styles.smartlinksHeading}>Actions:</h6>
          {smartlinks.map((smartlink, idx) => {
            return (
              <SmartLink
                key={`smartlink-${idx}`}
                smartlink={smartlink}
                integrationName={integr_name}
                integrationProject={project_path}
                integrationPath={integr_config_path}
                shouldBeDisabled={
                  smartlink.sl_enable_only_with_tool
                    ? !available.on_your_laptop
                    : false
                }
              />
            );
          })}
        </Flex>
        {config.host !== "web" && (
          <Button
            variant="soft"
            type="button"
            title={`Open ${integr_name}.yaml configuration file in your IDE`}
            onClick={() =>
              openFile({
                file_path: integr_config_path,
                line: 1,
              })
            }
          >
            Open {integr_name}.yaml
          </Button>
        )}
      </Flex>
    </Flex>
  );
};
