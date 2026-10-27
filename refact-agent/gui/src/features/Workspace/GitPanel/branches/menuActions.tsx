import { ContextMenu } from "@radix-ui/themes";
import { Fragment } from "react";

import { Menu } from "../../../../components/ui";
import styles from "./Branches.module.css";

export type GitMenuAction = {
  id: string;
  label: string;
  onSelect: () => void;
  disabled?: boolean;
  hint?: string;
  danger?: boolean;
  separatorBefore?: boolean;
};

export function ContextMenuActions({ actions }: { actions: GitMenuAction[] }) {
  return (
    <>
      {actions.map((action) => (
        <Fragment key={action.id}>
          {action.separatorBefore ? <ContextMenu.Separator /> : null}
          <ContextMenu.Item
            color={action.danger ? "red" : undefined}
            disabled={action.disabled}
            onSelect={action.onSelect}
            title={action.disabled ? action.hint : undefined}
          >
            {action.label}
          </ContextMenu.Item>
        </Fragment>
      ))}
    </>
  );
}

export function MenuActions({ actions }: { actions: GitMenuAction[] }) {
  return (
    <>
      {actions.map((action) => (
        <Fragment key={action.id}>
          {action.separatorBefore ? <Menu.Separator /> : null}
          <Menu.Item
            className={action.danger ? styles.dangerItem : undefined}
            disabled={action.disabled}
            onSelect={action.onSelect}
            title={action.disabled ? action.hint : undefined}
          >
            {action.label}
          </Menu.Item>
        </Fragment>
      ))}
    </>
  );
}
