import type { FC } from "react";
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

  if (fieldName === "when_isolated") return null;

  const label = toPascalCase(
    fieldName === "on_your_laptop" ? "enable" : "run_in_docker",
  );

  return (
    <Flex className={styles.availabilityToggle}>
      <Flex align="center" justify="between" gap="3">
        <Switch
          id={`switch-${fieldName}`}
          checked={value}
          onCheckedChange={handleSwitchChange}
        />
        <label
          className={styles.availabilityLabel}
          htmlFor={`switch-${fieldName}`}
        >
          {label}
        </label>
      </Flex>
    </Flex>
  );
};
