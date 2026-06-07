import { ArrowDown } from "lucide-react";
import { IconButton } from "../ui";
import styles from "./ScrollToBottomButton.module.css";

type ScrollToBottomButtonProps = {
  onClick: () => void;
};

export const ScrollToBottomButton = ({
  onClick,
}: ScrollToBottomButtonProps) => {
  return (
    <div className={styles.root}>
      <IconButton
        title="Follow stream"
        className={styles.button}
        onClick={onClick}
        aria-label="Follow stream"
        icon={ArrowDown}
        variant="soft"
      />
    </div>
  );
};
