import type { FC } from "react";
import { CustomLabel } from "../CustomFieldsAndWidgets";
import { toPascalCase } from "../../../utils/toPascalCase";
import { Flex, Switch } from "../../ui";

import styles from "./IntegrationForm.module.css";

type IntegrationAvailabilityProps = {
  fieldName: string;
  value: boolean;
  onChange: (fieldName: string, value: boolean) => void;
};

export const IntegrationAvailability: FC<IntegrationAvailabilityProps> = ({
  fieldName,
  value,
  onChange,
}) => {
  const handleSwitchChange = (checked: boolean) => {
    onChange(fieldName, checked);
  };

  // TODO: temporal solution to hide the switch for isolated mode
  if (fieldName === "when_isolated") return null;

  const handleLabelClick = () => {
    handleSwitchChange(value);
  };

  return (
    <Flex className={styles.availabilityToggle}>
      <Flex align="center" justify="between" gap="3">
        <Switch
          id={`switch-${fieldName}`}
          checked={value}
          onCheckedChange={handleSwitchChange}
        />
        <label htmlFor={`switch-${fieldName}`} onClick={handleLabelClick}>
          <CustomLabel
            label={toPascalCase(
              fieldName === "on_your_laptop" ? "enable" : "run_in_docker",
            )}
          />
        </label>
      </Flex>
    </Flex>
  );
};
