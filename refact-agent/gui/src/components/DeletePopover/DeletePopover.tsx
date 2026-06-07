import { FC } from "react";
import { Trash2 } from "lucide-react";
import classNames from "classnames";
import { Button, ButtonGroup, IconButton, Popover } from "../ui";
import styles from "./DeletePopover.module.css";

export type DeletePopoverProps = {
  isDisabled: boolean;
  isDeleting: boolean;
  itemName: string;
  deleteBy: string;
  handleDelete: (deleteBy: string) => void;
};

export const DeletePopover: FC<DeletePopoverProps> = ({
  deleteBy,
  itemName,
  handleDelete,
  isDeleting,
  isDisabled,
}) => {
  return (
    <Popover>
      <Popover.Trigger asChild>
        <IconButton
          aria-label="Delete configuration data"
          icon={Trash2}
          variant="danger"
          type="button"
          size="md"
          title="Delete configuration data"
          className={classNames({
            [styles.disabledButton]: isDeleting || isDisabled,
          })}
          disabled={isDeleting || isDisabled}
        />
      </Popover.Trigger>
      <Popover.Content maxWidth="360px">
        <div className={styles.content}>
          <div className={styles.copy}>
            <h4 className={styles.title}>Destructive action</h4>
            <p className={styles.description}>
              Do you really want to delete {itemName}&apos;s configuration data?
            </p>
          </div>

          <ButtonGroup>
            <Popover.Close asChild>
              <Button
                size="md"
                variant="danger"
                onClick={() => handleDelete(deleteBy)}
              >
                Delete
              </Button>
            </Popover.Close>
            <Popover.Close asChild>
              <Button size="md" variant="soft">Cancel</Button>
            </Popover.Close>
          </ButtonGroup>
        </div>
      </Popover.Content>
    </Popover>
  );
};
