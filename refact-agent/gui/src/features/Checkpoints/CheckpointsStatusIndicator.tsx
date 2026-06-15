import classNames from "classnames";
import { FileChangedStatus } from "./types";
import styles from "./CheckpointsStatusIndicator.module.css";

export const CheckpointsStatusIndicator = ({
  status,
}: {
  status: FileChangedStatus;
}) => {
  const shortenedStatus = status.split("")[0];

  return (
    <span className={classNames(styles.status, styles[status])}>
      {shortenedStatus}
    </span>
  );
};
