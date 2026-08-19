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
  size?: "sm" | "md";
  triggerClassName?: string;
};

export const DeletePopover: FC<DeletePopoverProps> = ({
  deleteBy,
  itemName,
  handleDelete,
  isDeleting,
  isDisabled,
  size = "md",
  triggerClassName,
}) => {
  return (
    <Popover>
      <Popover.Trigger asChild>
        <IconButton
          aria-label={`Delete ${itemName}`}
          icon={Trash2}
          variant="danger"
          type="button"
          size={size}
          title={`Delete ${itemName}`}
          className={classNames(triggerClassName, {
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
              Do you really want to delete {itemName}?
            </p>
          </div>

          <ButtonGroup className={styles.actions}>
            <Popover.Close asChild>
              <Button size="md" variant="soft">
                Cancel
              </Button>
            </Popover.Close>
            <Popover.Close asChild>
              <Button
                size="md"
                variant="danger"
                onClick={() => handleDelete(deleteBy)}
              >
                Delete
              </Button>
            </Popover.Close>
          </ButtonGroup>
        </div>
      </Popover.Content>
    </Popover>
  );
};
