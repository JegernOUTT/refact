import { AppRail } from "../../Apps/AppRail";
import classNames from "classnames";
import {
  Activity,
  Bot,
  Files,
  GitBranch,
  type LucideProps,
  type LucideIcon,
} from "lucide-react";
import { skipToken } from "@reduxjs/toolkit/query";
import { forwardRef, useCallback, useId } from "react";

import { Badge, IconButton, Tooltip } from "../../../components/ui";
import { RefactIcon } from "../../../images/RefactIcon";
import { BuddyRailButton } from "../../Buddy/Companion/BuddyRailButton";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import { useGetGitStatusQuery } from "../../../services/refact/gitRead";
import {
  closeThread,
  selectActiveBackgroundAgents,
  selectAllThreads,
  selectChatId,
} from "../../Chat/Thread";
import { popBackTo, selectCurrentPage } from "../../Pages/pagesSlice";
import { collectTabIds } from "../../ChatPanes/panesTree";
import { isFileSurface } from "../surfaceKey";
import { selectCapabilities, selectHost } from "../../Config/configSlice";
import { changedFileCount } from "../GitPanel";
import { activeChatsNeedAttention } from "../Dock/activeChatsModel";
import { useActiveChats } from "../Dock/useActiveChats";
import {
  selectFocusedChatWorkspaceRoots,
  selectFocusedWorkspaceChatId,
  selectPanelsForced,
  selectTabs,
  selectWorkspaceGroups,
  selectWorkspaceFocusedChatId,
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

const HomeLogo = forwardRef<SVGSVGElement, LucideProps>(function HomeLogo(
  { className, ...props },
  _ref,
) {
  return (
    <RefactIcon
      {...props}
      className={classNames(className, styles.homeLogo, "rf-status-pulse")}
      stroke="none"
    />
  );
});

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
    id: "activity",
    icon: Activity,
    label: "Active chats",
    shortcut: `${modifierLabel}4`,
  },
];

const countDescriptionFor = (
  section: WorkspaceDockSection,
  count: number,
): string => {
  if (section === "git") {
    return `${count} changed ${count === 1 ? "file" : "files"}`;
  }
  if (section === "activity") {
    return `${count} active ${count === 1 ? "chat" : "chats"}`;
  }
  return `${count} active ${count === 1 ? "agent" : "agents"}`;
};

export function ActivityRail() {
  const railId = useId();
  const dispatch = useAppDispatch();
  const page = useAppSelector(selectCurrentPage);
  const allThreads = useAppSelector(selectAllThreads);
  const currentChatId = useAppSelector(selectChatId);
  const workspaceTabs = useAppSelector(selectTabs);
  const workspaceGroups = useAppSelector(selectWorkspaceGroups);
  const hasOpenFiles =
    workspaceTabs.some(isFileSurface) ||
    Object.values(workspaceGroups).some((group) =>
      group ? collectTabIds(group.root).some(isFileSurface) : false,
    );
  const host = useAppSelector(selectHost);
  const capabilities = useAppSelector(selectCapabilities);
  const panelsForced = useAppSelector(selectPanelsForced);
  const dock = useAppSelector(selectWorkspaceDock);
  const focusedChatId = useAppSelector(selectFocusedWorkspaceChatId);
  const homeChatId = useAppSelector(selectWorkspaceFocusedChatId);
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
  const activeChats = useActiveChats(availability.activity);
  const activeChatCount = activeChats.entries.length;
  const activeChatsAttention = activeChatsNeedAttention(activeChats.entries);
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

  const goHome = useCallback(() => {
    if (page?.name === "chat") {
      const chatId = homeChatId ?? currentChatId;
      const currentThread = allThreads[chatId];
      if (
        currentThread &&
        currentThread.thread.messages.length === 0 &&
        !hasOpenFiles
      ) {
        dispatch(closeThread({ id: chatId }));
      }
    }
    dispatch(popBackTo({ name: "history" }));
  }, [page, homeChatId, currentChatId, allThreads, hasOpenFiles, dispatch]);

  const handleSelect = useCallback(
    (section: WorkspaceDockSection) => {
      const showing =
        dock.open &&
        dock.section === section &&
        isDockSectionAvailable(section, availability);
      if (showing) {
        dispatch(setDockOpen(false));
        return;
      }
      if (!isDockSectionAvailable(section, availability)) {
        dispatch(setPanelsForced(true));
      }
      dispatch(setDockSection(section));
      dispatch(setDockOpen(true));
    },
    [availability, dispatch, dock.open, dock.section],
  );

  return (
    <nav aria-label="Workspace sections" className={styles.rail}>
      <div className={styles.slot}>
        <Tooltip content="Home">
          <IconButton
            aria-label="Home"
            aria-pressed={page?.name === "history"}
            className={styles.button}
            icon={HomeLogo}
            onClick={goHome}
            size="sm"
            variant="ghost"
          />
        </Tooltip>
      </div>
      {RAIL_ITEMS.map((item) => {
        const badgeCount =
          item.id === "agents"
            ? agentCount
            : item.id === "git"
              ? changedCount
              : item.id === "activity"
                ? activeChatCount
                : 0;
        const activitySuffix =
          item.id !== "activity"
            ? ""
            : `${activeChatsAttention ? ", needs attention" : ""}${
                activeChats.staleReason === "offline"
                  ? ", last known (engine offline)"
                  : activeChats.staleReason === "refresh_failed"
                    ? ", last known (refresh failed)"
                    : ""
              }`;
        const countDescription = `${countDescriptionFor(
          item.id,
          badgeCount,
        )}${activitySuffix}`;
        const descriptionId = `${railId}-${item.id}-count`;
        return (
          <div className={styles.slot} key={item.id}>
            <Tooltip
              content={`${item.label} ${item.shortcut}${
                badgeCount > 0 ? ` — ${countDescription}` : ""
              }`}
            >
              <IconButton
                aria-describedby={badgeCount > 0 ? descriptionId : undefined}
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
                tone={
                  item.id === "activity" && activeChatsAttention
                    ? "danger"
                    : "accent"
                }
              >
                {badgeCount > 99 ? "99+" : badgeCount}
              </Badge>
            ) : null}
            {badgeCount > 0 ? (
              <span className={styles.srOnly} id={descriptionId}>
                {countDescription}
              </span>
            ) : null}
          </div>
        );
      })}
      <AppRail className={styles.button} />
      <BuddyRailButton className={styles.bottomSlot} />
    </nav>
  );
}
