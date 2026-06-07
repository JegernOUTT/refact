import React, { useMemo } from "react";
import {
  BarChart3,
  Bug,
  FileText,
  Gauge,
  Keyboard,
  Menu as MenuIcon,
  Puzzle,
  Rocket,
  Settings,
  SlidersHorizontal,
  Sparkles,
  Star,
} from "lucide-react";

import { selectHost, type Config } from "../../features/Config/configSlice";
import { useAppSelector, useEventsBusForIDE } from "../../hooks";
import { useOpenUrl } from "../../hooks/useOpenUrl";
import { Icon, IconButton, Menu, Tooltip } from "../ui";
import styles from "./Toolbar.module.css";

export type DropdownNavigationOptions =
  | "stats"
  | "settings"
  | "hot keys"
  | "integrations"
  | "providers"
  | "knowledge graph"
  | "customization"
  | "default models"
  | "extensions"
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
        <Menu.Item onSelect={() => handleNavigation("integrations")}>
          <Icon icon={Puzzle} size="sm" /> Set up Agent Integrations
        </Menu.Item>
        <Menu.Item onSelect={() => handleNavigation("providers")}>
          <Icon icon={SlidersHorizontal} size="sm" /> Configure Providers
        </Menu.Item>
        <Menu.Item onSelect={() => handleNavigation("default models")}>
          <Icon icon={Star} size="sm" /> Default Models
        </Menu.Item>
        <Menu.Item onSelect={() => handleNavigation("knowledge graph")}>
          <Icon icon={Gauge} size="sm" /> Manage Knowledge
        </Menu.Item>
        <Menu.Item onSelect={() => handleNavigation("settings")}>
          <Icon icon={Settings} size="sm" /> {refactProductType} Settings
        </Menu.Item>
        <Menu.Item onSelect={() => handleNavigation("hot keys")}>
          <Icon icon={Keyboard} size="sm" /> IDE Hotkeys
        </Menu.Item>
        <Menu.Item onSelect={() => handleNavigation("customization")}>
          <Icon icon={Rocket} size="sm" /> Customize Modes & Agents
        </Menu.Item>
        <Menu.Item onSelect={() => handleNavigation("extensions")}>
          <Icon icon={Sparkles} size="sm" /> Skills, Commands & Hooks
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
