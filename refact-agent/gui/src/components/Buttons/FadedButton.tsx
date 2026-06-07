import React from "react";
import classNames from "classnames";
import { Button, type ButtonProps } from "../ui";
import styles from "./button.module.css";

export type FadedButtonProps = ButtonProps;

type LegacyFadedButtonProps = FadedButtonProps & {
  color?: string;
  mx?: string;
};

export const FadedButton: React.FC<LegacyFadedButtonProps> = ({
  color: _color,
  mx: _mx,
  ...props
}) => {
  return (
    <Button
      variant="plain"
      {...props}
      className={classNames(styles.button_faded, props.className)}
    />
  );
};
