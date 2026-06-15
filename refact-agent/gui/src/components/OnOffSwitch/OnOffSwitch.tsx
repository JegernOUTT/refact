import React, { MouseEventHandler } from "react";
import { Switch } from "../ui";

export type OnOffSwitchProps = {
  isEnabled: boolean;
  isUnavailable?: boolean;
  isUpdating?: boolean;
  handleClick: MouseEventHandler<HTMLButtonElement | HTMLDivElement>;
};

export const OnOffSwitch: React.FC<OnOffSwitchProps> = ({
  isEnabled,
  isUnavailable = false,
  isUpdating = false,
  handleClick,
}) => {
  return (
    <Switch
      aria-label={isEnabled ? "Turn off" : "Turn on"}
      checked={isEnabled}
      disabled={isUpdating || isUnavailable}
      onClick={handleClick}
    />
  );
};
