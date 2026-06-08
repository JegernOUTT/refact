import type { FC } from "react";
import classNames from "classnames";
import { ArrowLeft } from "lucide-react";
import { Button } from "../../ui";
import styles from "./IntegrationsHeader.module.css";
import { getIntegrationInfo } from "../../../utils/getIntegrationInfo";

type IntegrationsHeaderProps = {
  handleFormReturn: () => void;
  integrationName: string;
  icon: string;
  instantBackReturn?: boolean;
  handleInstantReturn?: () => void;
  embedded?: boolean;
};

export const IntegrationsHeader: FC<IntegrationsHeaderProps> = ({
  handleFormReturn,
  integrationName,
  icon,
  instantBackReturn = false,
  handleInstantReturn,
  embedded,
}) => {
  const handleButtonClick = () => {
    if (instantBackReturn && handleInstantReturn) {
      handleInstantReturn();
    } else {
      handleFormReturn();
    }
  };

  const { displayName } = getIntegrationInfo(integrationName);

  return (
    <div className={classNames(styles.header, { [styles.fixed]: !embedded })}>
      <div className={styles.row}>
        <Button size="sm" variant="soft" leftIcon={ArrowLeft} onClick={handleButtonClick}>
          {instantBackReturn ? "Back to chat" : "Configurations"}
        </Button>
        <div className={styles.info}>
          <img src={icon} className={styles.icon} alt={integrationName} />
          <h5 className={styles.name}>{displayName}</h5>
        </div>
      </div>
    </div>
  );
};
