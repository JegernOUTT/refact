import React, { useMemo } from "react";
import {
  BarChart3,
  Bug,
  FileText,
  Gauge,
  Menu as MenuIcon,
  Settings,
  SlidersHorizontal,
} from "lucide-react";

import { selectHost, type Config } from "../../features/Config/configSlice";
import { useAppSelector, useEventsBusForIDE } from "../../hooks";
import { useOpenUrl } from "../../hooks/useOpenUrl";
import { Icon, IconButton, Menu, Tooltip } from "../ui";
import styles from "./Toolbar.module.css";

export type DropdownNavigationOptions =
  | "stats"
  | "settings"
  | "knowledge graph"
  | "general settings"
  | "";

type DropdownProps = {
  handleNavigation: (to: DropdownNavigationOptions) => void;
  triggerClassName?: string;
  useGhostTrigger?: boolean;
};

function linkForBugReports(_host: Config["host"]): string {
  return "https://github.com/JegernOUTT/refact/issues";
}

export const Dropdown: React.FC<DropdownProps> = ({
  handleNavigation,
  triggerClassName,
}: DropdownProps) => {
  const host = useAppSelector(selectHost);
  const bugUrl = linkForBugReports(host);
  const openUrl = useOpenUrl();
  const { openPrivacyFile } = useEventsBusForIDE();

  const refactProductType = useMemo(() => {
    if (host === "jetbrains") return "Plugin";
    return "Extension";
  }, [host]);
  return (
    <Menu>
      <Tooltip>
        <Tooltip.Trigger asChild>
          <Menu.Trigger asChild>
            <IconButton
              aria-label="Menu"
              className={triggerClassName ?? styles.iconButton}
              icon={MenuIcon}
              size="sm"
              variant="plain"
            />
          </Menu.Trigger>
        </Tooltip.Trigger>
        <Tooltip.Content side="bottom">Menu</Tooltip.Content>
      </Tooltip>

      <Menu.Content>
        <Menu.Item onSelect={() => handleNavigation("general settings")}>
          <Icon icon={Settings} size="sm" /> Settings
        </Menu.Item>
        <Menu.Item onSelect={() => handleNavigation("knowledge graph")}>
          <Icon icon={Gauge} size="sm" /> Manage Knowledge
        </Menu.Item>
        <Menu.Item onSelect={() => handleNavigation("settings")}>
          <Icon icon={SlidersHorizontal} size="sm" /> {refactProductType} Settings
        </Menu.Item>
        <Menu.Item onSelect={() => void openPrivacyFile()}>
          <Icon icon={FileText} size="sm" /> Edit privacy.yaml
        </Menu.Item>
        <Menu.Separator />
        <Menu.Item
          onSelect={(event) => {
            event.preventDefault();
            openUrl(bugUrl);
          }}
        >
          <Icon icon={Bug} size="sm" /> Report a bug
        </Menu.Item>
        <Menu.Item onSelect={() => handleNavigation("stats")}>
          <Icon icon={BarChart3} size="sm" /> Usage Dashboard
        </Menu.Item>
      </Menu.Content>
    </Menu>
  );
};
