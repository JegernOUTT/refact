import type { FC, KeyboardEventHandler, MouseEventHandler } from "react";
import classNames from "classnames";

import { Surface } from "../../ui";
import { useAppSelector } from "../../../hooks";
import { useUpdateIntegration } from "./useUpdateIntegration";

import {
  IntegrationWithIconRecord,
  NotConfiguredIntegrationWithIconRecord,
} from "../../../services/refact";

import { selectConfig } from "../../../features/Config/configSlice";
import { formatIntegrationIconPath } from "../../../utils/formatIntegrationIconPath";
import { getIntegrationInfo } from "../../../utils/getIntegrationInfo";
import { buildApiUrl } from "../../../services/refact/apiUrl";

import styles from "./IntegrationCard.module.css";
import { OnOffSwitch } from "../../OnOffSwitch/OnOffSwitch";

type IntegrationCardProps = {
  integration:
    | IntegrationWithIconRecord
    | NotConfiguredIntegrationWithIconRecord;
  handleIntegrationShowUp: (
    integration:
      | IntegrationWithIconRecord
      | NotConfiguredIntegrationWithIconRecord,
  ) => void;
  isNotConfigured?: boolean;
};

export const IntegrationCard: FC<IntegrationCardProps> = ({
  integration,
  handleIntegrationShowUp,
  isNotConfigured = false,
}) => {
  const config = useAppSelector(selectConfig);

  const iconPath = formatIntegrationIconPath(integration.icon_path);
  const integrationIconPath = iconPath.startsWith("/v1/")
    ? iconPath
    : `/v1${iconPath}`;
  const integrationLogo = buildApiUrl(config, integrationIconPath);

  const { displayName } = getIntegrationInfo(integration.integr_name);
  const {
    updateIntegrationAvailability,
    integrationAvailability,
    isUpdatingAvailability,
  } = useUpdateIntegration({ integration });

  const handleAvailabilityClick: MouseEventHandler<HTMLDivElement> = (
    event,
  ) => {
    if (isUpdatingAvailability) return;
    event.stopPropagation();
    void updateIntegrationAvailability();
  };

  const openIntegration = () => {
    if (isUpdatingAvailability) return;
    handleIntegrationShowUp(integration);
  };

  const handleCardKeyDown: KeyboardEventHandler<HTMLDivElement> = (event) => {
    if (event.target !== event.currentTarget) return;
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    openIntegration();
  };

  return (
    <Surface
      animated="rise"
      radius="card"
      variant="plain"
      interactive
      role="button"
      tabIndex={0}
      className={classNames(styles.integrationCard, {
        [styles.integrationCardInline]: isNotConfigured,
        [styles.disabledCard]: isUpdatingAvailability,
      })}
      onClick={openIntegration}
      onKeyDown={handleCardKeyDown}
    >
      <span
        className={classNames(styles.content, {
          [styles.contentInline]: isNotConfigured,
        })}
      >
        <img
          src={integrationLogo}
          className={styles.integrationIcon}
          alt={integration.integr_name}
        />
        <span
          className={classNames(styles.body, {
            [styles.bodyInline]: isNotConfigured,
          })}
        >
          <span
            className={classNames(styles.title, {
              [styles.titleInline]: isNotConfigured,
            })}
          >
            {displayName}
          </span>
          {!isNotConfigured && (
            <span className={styles.switchWrap}>
              <OnOffSwitch
                isEnabled={integrationAvailability.on_your_laptop}
                handleClick={handleAvailabilityClick}
              />
            </span>
          )}
        </span>
      </span>
    </Surface>
  );
};
