import React, { useMemo } from "react";
import {
  BarChart3,
  Bug,
  CheckSquare,
  FileText,
  Gauge,
  Menu as MenuIcon,
  Moon,
  Settings,
  SlidersHorizontal,
  Sun,
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
  isDarkMode?: boolean;
  onCreateNewTask?: () => void;
  onToggleDarkMode?: () => void;
  triggerClassName?: string;
  useGhostTrigger?: boolean;
};

function linkForBugReports(_host: Config["host"]): string {
  return "https://github.com/JegernOUTT/refact/issues";
}

export const Dropdown: React.FC<DropdownProps> = ({
  handleNavigation,
  isDarkMode = false,
  onCreateNewTask,
  onToggleDarkMode,
  triggerClassName,
}: DropdownProps) => {
  const host = useAppSelector(selectHost);
  const bugUrl = linkForBugReports(host);
  const openUrl = useOpenUrl();
  const { openPrivacyFile } = useEventsBusForIDE();
  const hasSecondaryActions = [onCreateNewTask, onToggleDarkMode].some(
    (action) => action !== undefined,
  );

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
        {onCreateNewTask && (
          <Menu.Item onSelect={() => onCreateNewTask()}>
            <Icon icon={CheckSquare} size="sm" /> New Task
          </Menu.Item>
        )}
        {onToggleDarkMode && (
          <Menu.Item onSelect={() => onToggleDarkMode()}>
            <Icon icon={isDarkMode ? Moon : Sun} size="sm" /> Toggle Dark Mode
          </Menu.Item>
        )}
        {hasSecondaryActions && <Menu.Separator />}
        <Menu.Item onSelect={() => handleNavigation("general settings")}>
          <Icon icon={Settings} size="sm" /> Settings
        </Menu.Item>
        <Menu.Item onSelect={() => handleNavigation("knowledge graph")}>
          <Icon icon={Gauge} size="sm" /> Manage Knowledge
        </Menu.Item>
        <Menu.Item onSelect={() => handleNavigation("settings")}>
          <Icon icon={SlidersHorizontal} size="sm" /> {refactProductType}{" "}
          Settings
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
