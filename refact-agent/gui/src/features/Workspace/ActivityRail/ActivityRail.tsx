import classNames from "classnames";
import {
  Bot,
  CheckSquare,
  Files,
  GitBranch,
  type LucideIcon,
} from "lucide-react";
import { skipToken } from "@reduxjs/toolkit/query";
import { useCallback } from "react";

import { Badge, IconButton, Tooltip } from "../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import { useGetGitStatusQuery } from "../../../services/refact/gitRead";
import {
  agentsSectionUserClosed,
  agentsSectionUserOpened,
} from "../../AgentsPanel/agentsPanelSlice";
import { selectActiveBackgroundAgents } from "../../Chat/Thread";
import { selectCapabilities, selectHost } from "../../Config/configSlice";
import { changedFileCount } from "../GitPanel";
import {
  selectFocusedChatWorkspaceRoots,
  selectFocusedWorkspaceChatId,
  selectPanelsForced,
  selectWorkspaceDock,
  setDockOpen,
  setDockSection,
  setPanelsForced,
  type WorkspaceDockSection,
} from "../workspaceSlice";
import {
  isDockSectionAvailable,
  resolveWorkspaceDockAvailability,
} from "../workspaceAvailability";
import styles from "./ActivityRail.module.css";

const isMac =
  typeof navigator !== "undefined" &&
  /Mac|iPod|iPhone|iPad/.test(navigator.platform);

const modifierLabel = isMac ? "⌘" : "Ctrl+";

type RailItem = {
  id: WorkspaceDockSection;
  icon: LucideIcon;
  label: string;
  shortcut: string;
};

const RAIL_ITEMS: RailItem[] = [
  { id: "files", icon: Files, label: "Files", shortcut: `${modifierLabel}1` },
  { id: "git", icon: GitBranch, label: "Git", shortcut: `${modifierLabel}2` },
  { id: "agents", icon: Bot, label: "Agents", shortcut: `${modifierLabel}3` },
  {
    id: "tasks",
    icon: CheckSquare,
    label: "Tasks",
    shortcut: `${modifierLabel}4`,
  },
];

export function ActivityRail() {
  const dispatch = useAppDispatch();
  const host = useAppSelector(selectHost);
  const capabilities = useAppSelector(selectCapabilities);
  const panelsForced = useAppSelector(selectPanelsForced);
  const dock = useAppSelector(selectWorkspaceDock);
  const focusedChatId = useAppSelector(selectFocusedWorkspaceChatId);
  const contextRoots = useAppSelector(selectFocusedChatWorkspaceRoots);
  const availability = resolveWorkspaceDockAvailability(
    host,
    capabilities,
    panelsForced,
  );
  const { data: gitStatus } = useGetGitStatusQuery(
    availability.git ? contextRoots : skipToken,
  );
  const activeAgents = useAppSelector((state) =>
    focusedChatId ? selectActiveBackgroundAgents(state, focusedChatId) : null,
  );
  const changedCount =
    gitStatus?.roots.reduce(
      (count, root) => count + changedFileCount(root),
      0,
    ) ?? 0;
  const agentCount = activeAgents?.length ?? 0;
  const isShowing = (section: WorkspaceDockSection) =>
    dock.open &&
    dock.section === section &&
    isDockSectionAvailable(section, availability);

  const handleSelect = useCallback(
    (section: WorkspaceDockSection) => {
      const showing =
        dock.open &&
        dock.section === section &&
        isDockSectionAvailable(section, availability);
      if (showing) {
        if (section === "agents" && focusedChatId) {
          dispatch(agentsSectionUserClosed(focusedChatId));
        }
        dispatch(setDockOpen(false));
        return;
      }
      if (!isDockSectionAvailable(section, availability)) {
        dispatch(setPanelsForced(true));
      }
      if (section === "agents" && focusedChatId) {
        dispatch(agentsSectionUserOpened(focusedChatId));
      }
      dispatch(setDockSection(section));
      dispatch(setDockOpen(true));
    },
    [availability, dispatch, dock.open, dock.section, focusedChatId],
  );

  return (
    <nav aria-label="Workspace sections" className={styles.rail}>
      {RAIL_ITEMS.map((item) => {
        const badgeCount =
          item.id === "agents"
            ? agentCount
            : item.id === "git"
              ? changedCount
              : 0;
        return (
          <div className={styles.slot} key={item.id}>
            <Tooltip content={`${item.label} ${item.shortcut}`}>
              <IconButton
                aria-label={item.label}
                aria-pressed={isShowing(item.id)}
                className={styles.button}
                icon={item.icon}
                onClick={() => handleSelect(item.id)}
                size="sm"
                variant="ghost"
              />
            </Tooltip>
            {badgeCount > 0 ? (
              <Badge
                aria-hidden="true"
                className={classNames(styles.badge, "rf-enter-scale")}
                key={badgeCount}
                size="xs"
                tone="accent"
              >
                {badgeCount}
              </Badge>
            ) : null}
          </div>
        );
      })}
    </nav>
  );
}
