import React, { useMemo } from "react";
import { selectHost, type Config } from "../../features/Config/configSlice";
import { useAppSelector, useEventsBusForIDE } from "../../hooks";
import { useOpenUrl } from "../../hooks/useOpenUrl";
import { DropdownMenu, HoverCard, Text } from "@radix-ui/themes";
import {
  BarChartIcon,
  ExclamationTriangleIcon,
  FileTextIcon,
  GearIcon,
  HamburgerMenuIcon,
  KeyboardIcon,
  LightningBoltIcon,
  MixerHorizontalIcon,
  ReaderIcon,
  RocketIcon,
  StarIcon,
} from "@radix-ui/react-icons";
import styles from "./Toolbar.module.css";
import { PuzzleIcon } from "../../images/PuzzleIcon";

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
    <DropdownMenu.Root>
      <HoverCard.Root>
        <HoverCard.Trigger>
          <DropdownMenu.Trigger>
            <button
              type="button"
              className={triggerClassName ?? styles.iconButton}
              aria-label="Menu"
            >
              <HamburgerMenuIcon />
            </button>
          </DropdownMenu.Trigger>
        </HoverCard.Trigger>
        <HoverCard.Content size="1" side="bottom">
          <Text as="p" size="2">
            Menu
          </Text>
        </HoverCard.Content>
      </HoverCard.Root>

      <DropdownMenu.Content>
        <DropdownMenu.Item onSelect={() => handleNavigation("integrations")}>
          <PuzzleIcon /> Set up Agent Integrations
        </DropdownMenu.Item>
        <DropdownMenu.Item onSelect={() => handleNavigation("providers")}>
          <MixerHorizontalIcon /> Configure Providers
        </DropdownMenu.Item>
        <DropdownMenu.Item onSelect={() => handleNavigation("default models")}>
          <StarIcon /> Default Models
        </DropdownMenu.Item>
        <DropdownMenu.Item onSelect={() => handleNavigation("knowledge graph")}>
          <ReaderIcon /> Manage Knowledge
        </DropdownMenu.Item>
        <DropdownMenu.Item onSelect={() => handleNavigation("settings")}>
          <GearIcon /> {refactProductType} Settings
        </DropdownMenu.Item>
        <DropdownMenu.Item onSelect={() => handleNavigation("hot keys")}>
          <KeyboardIcon /> IDE Hotkeys
        </DropdownMenu.Item>
        <DropdownMenu.Item onSelect={() => handleNavigation("customization")}>
          <RocketIcon /> Customize Modes & Agents
        </DropdownMenu.Item>
        <DropdownMenu.Item onSelect={() => handleNavigation("extensions")}>
          <LightningBoltIcon /> Skills, Commands & Hooks
        </DropdownMenu.Item>
        <DropdownMenu.Item onSelect={() => void openPrivacyFile()}>
          <FileTextIcon /> Edit privacy.yaml
        </DropdownMenu.Item>
        <DropdownMenu.Separator />
        <DropdownMenu.Item
          onSelect={(event) => {
            event.preventDefault();
            openUrl(bugUrl);
          }}
        >
          <ExclamationTriangleIcon /> Report a bug
        </DropdownMenu.Item>
        <DropdownMenu.Item onSelect={() => handleNavigation("stats")}>
          <BarChartIcon /> Usage Dashboard
        </DropdownMenu.Item>
      </DropdownMenu.Content>
    </DropdownMenu.Root>
  );
};
