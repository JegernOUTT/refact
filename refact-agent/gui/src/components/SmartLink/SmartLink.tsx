import { useCallback } from "react";
import type { FC } from "react";
import { WandSparkles } from "lucide-react";
import type { SmartLink as SmartLinkType } from "../../services/refact";
import { Button } from "../ui";
import { useSmartLinks } from "../../hooks";
import styles from "./SmartLink.module.css";

export const SmartLink: FC<{
  smartlink: SmartLinkType;
  integrationName: string;
  integrationPath: string;
  integrationProject: string;
  isSmall?: boolean;
  shouldBeDisabled?: boolean;
}> = ({
  smartlink,
  integrationName,
  integrationPath,
  integrationProject,
  isSmall = false,
  shouldBeDisabled,
}) => {
  const { handleGoTo, handleSmartLink } = useSmartLinks();

  const { sl_goto, sl_chat } = smartlink;

  const handleClick = useCallback(() => {
    if (sl_goto) {
      handleGoTo({ goto: sl_goto });
      return;
    }
    if (sl_chat) {
      handleSmartLink(
        sl_chat,
        integrationName,
        integrationPath,
        integrationProject,
      );
    }
  }, [
    sl_goto,
    sl_chat,
    handleGoTo,
    handleSmartLink,
    integrationName,
    integrationPath,
    integrationProject,
  ]);

  const title = sl_chat?.reduce<string[]>((acc, cur) => {
    if (typeof cur.content === "string")
      return [...acc, `${cur.role}: ${cur.content}`];
    return acc;
  }, []);

  return (
    <Button
      size={isSmall ? "sm" : "md"}
      onClick={handleClick}
      title={title ? title.join("\n") : ""}
      type="button"
      variant="soft"
      leftIcon={smartlink.sl_chat ? WandSparkles : undefined}
      className={styles.magicButton}
      disabled={shouldBeDisabled}
    >
      {smartlink.sl_label}
    </Button>
  );
};
