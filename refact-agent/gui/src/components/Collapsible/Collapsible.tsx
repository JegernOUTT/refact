import React from "react";
import * as RadixCollapsible from "@radix-ui/react-collapsible";
import { Rows3, X } from "lucide-react";
import classNames from "classnames";
import { Button } from "../ui";
import styles from "./collapsible.module.css";

export type CollapsibleProps = Pick<
  RadixCollapsible.CollapsibleProps,
  "disabled" | "className" | "defaultOpen"
> &
  React.PropsWithChildren<{
    className?: string;
    disabled?: boolean;
    title?: React.ReactNode;
  }>;

export const Collapsible: React.FC<CollapsibleProps> = ({
  children,
  title,
  className,
  ...props
}) => {
  const [open, setOpen] = React.useState(props.defaultOpen ?? false);
  const TriggerIcon = open ? X : Rows3;

  return (
    <RadixCollapsible.Root
      {...props}
      className={classNames(styles.root, className)}
      open={open}
      onOpenChange={setOpen}
    >
      <div className={styles.header}>
        <RadixCollapsible.Trigger asChild>
          <Button
            className={styles.trigger}
            disabled={props.disabled}
            rightIcon={TriggerIcon}
            size="sm"
            variant="ghost"
          >
            {title}
          </Button>
        </RadixCollapsible.Trigger>
      </div>

      <RadixCollapsible.Content className={styles.content}>
        <div className={styles.contentInner}>{children}</div>
      </RadixCollapsible.Content>
    </RadixCollapsible.Root>
  );
};
