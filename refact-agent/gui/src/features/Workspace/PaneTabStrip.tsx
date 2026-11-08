import { useConfig } from "../../hooks/useConfig";
import {
  contentRouteForSurface,
  contentRouteHref,
  isPrimaryContentClick,
} from "../../utils/contentRoutes";
import { ChatActivityStatus } from "../../components/ChatActivity/ChatActivityStatus";
import { activityFromSessionState } from "../../utils/chatActivity";
import classNames from "classnames";
import {
  Columns3,
  Rows3,
  Ellipsis,
  X,
  FileDiff,
  FileText,
  GitBranch,
  Palette,
} from "lucide-react";
import { useMemo, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Badge, Icon, IconButton, Menu, Tooltip } from "../../components/ui";
import { useAppDispatch } from "../../hooks";
import { useTabDisplay, type TabDisplayInfo } from "./tabDisplay";
import { TabHoverDetails } from "./TabHoverDetails";
import { useTabMotion } from "./useTabMotion";
import type { LeafPane } from "../ChatPanes/panesTree";
import { beginPointerDragGesture } from "../ChatPanes/pointerDrag";
import {
  hasSplittableTabDrag,
  readTabDragSurfaceKey,
  setTabDragData,
  surfaceKeyFromTabDragPayload,
  type TabDragPayload,
} from "../ChatPanes/tabDrag";
import {
  usePointerDragHost,
  usePointerDropZone,
} from "../ChatPanes/usePointerDrag";
import { isSplittableSurface, type SurfaceKey } from "./surfaceKey";
import {
  addSurfaceToPane,
  closePane,
  closePaneTab,
  closeTab,
  setActiveTab,
  focusPane,
  reorderPaneTabs,
  setPaneActive,
  splitTab,
} from "./workspaceSlice";
import { PaneHeaderActions } from "./PaneHeaderContext";
import { panePanelId, paneTabId } from "./paneTabIds";
import { fitPaneTabs, type PaneTabFitChange } from "./paneTabFit";
import styles from "./PaneTabStrip.module.css";

type LocalTab = {
  key: SurfaceKey;
  title: string;
  file: boolean;
  display: TabDisplayInfo;
};
export type PaneNavigationEntry = {
  key: string;
  surfaceKey: SurfaceKey;
  display?: TabDisplayInfo;
  onActivate: () => void;
  onClose?: () => void;
  controlsId?: string;
  onDropSurface?: (key: SurfaceKey) => void;
};
type RenderTab = LocalTab & { navigation?: PaneNavigationEntry };
export type PaneTabOwner = { tabId: SurfaceKey; leaf: LeafPane };

function TabDisplayContent({
  display,
  titleClassName,
}: {
  display: TabDisplayInfo;
  titleClassName?: string;
}) {
  const icon =
    display.kind === "file"
      ? FileText
      : display.kind === "gitdiff"
        ? FileDiff
        : display.kind === "git"
          ? GitBranch
          : display.kind === "design"
            ? Palette
            : null;
  return (
    <>
      {icon ? (
        <Icon icon={icon} size="sm" tone="muted" />
      ) : (
        <ChatActivityStatus
          activity={
            display.activity ?? activityFromSessionState(display.session_state)
          }
          compact
        />
      )}
      {(display.kind === "buddy" || display.is_buddy_chat) &&
        display.title.trim().toLowerCase() !== "buddy" && (
          <Badge tone="accent" size="xs" variant="outline">
            Buddy
          </Badge>
        )}
      {(display.kind === "task" || display.is_task_chat) &&
        display.title.trim().toLowerCase() !== "task" && (
          <Badge tone="muted" size="xs" variant="outline">
            Task
          </Badge>
        )}
      <span className={titleClassName}>{display.title}</span>
      {display.unreadNotificationCount > 0 && (
        <Badge
          tone="warning"
          size="xs"
          aria-label={`${display.unreadNotificationCount} unread process notifications`}
        >
          {display.unreadNotificationCount > 9
            ? "9+"
            : display.unreadNotificationCount}
        </Badge>
      )}
    </>
  );
}

function restorePaneTabFocus(workspaceOnly = false) {
  requestAnimationFrame(() => {
    const fallback = document.querySelector<HTMLElement>(
      workspaceOnly
        ? '[aria-label="Workspace tabs"] [role="tab"][aria-selected="true"]'
        : '[role="tablist"] [role="tab"][aria-selected="true"]',
    );
    fallback?.focus();
  });
}

function LocalTabButton({
  tab,
  leaf,
  tabId,
  active,
  activate,
  keys,
  close,
  focusable,
  headerLeafId = leaf.id,
  hasOwner = true,
  minimumWidth,
}: {
  tab: RenderTab;
  leaf: LeafPane;
  tabId: SurfaceKey;
  active: boolean;
  activate: (key: SurfaceKey, focus?: boolean) => void;
  keys: string[];
  close: (tab: RenderTab) => void;
  focusable: boolean;
  headerLeafId?: string;
  hasOwner?: boolean;
  minimumWidth?: number;
}) {
  const dispatch = useAppDispatch();
  const pointerEnabled = usePointerDragHost();
  const config = useConfig();
  const route = contentRouteForSurface(tab.key);
  const href =
    config.host === "web" && route ? contentRouteHref(route) : undefined;
  const cleanup = useRef<() => void>();
  useEffect(() => () => cleanup.current?.(), []);
  const drop = (key: SurfaceKey | null) => {
    if (tab.navigation !== undefined) {
      if (key) tab.navigation.onDropSurface?.(key);
      return;
    }
    if (!hasOwner || !key || !isSplittableSurface(key)) return;
    const fromIndex = leaf.tabIds.indexOf(key);
    if (fromIndex >= 0) {
      dispatch(
        reorderPaneTabs({
          tabId,
          leafId: leaf.id,
          fromIndex,
          toIndex: leaf.tabIds.indexOf(tab.key),
        }),
      );
    } else {
      dispatch(addSurfaceToPane({ tabId, leafId: leaf.id, surfaceKey: key }));
      dispatch(
        reorderPaneTabs({
          tabId,
          leafId: leaf.id,
          fromIndex: leaf.tabIds.length,
          toIndex: leaf.tabIds.indexOf(tab.key),
        }),
      );
    }
  };
  const canDrop = tab.navigation ? !!tab.navigation.onDropSurface : hasOwner;
  const surfaceKey = tab.navigation?.surfaceKey ?? tab.key;
  const pointerDrop = usePointerDropZone({
    enabled: pointerEnabled && canDrop,
    accepts: (payload: TabDragPayload) => {
      const key = surfaceKeyFromTabDragPayload(payload);
      return (
        canDrop &&
        key !== surfaceKey &&
        key !== null &&
        isSplittableSurface(key)
      );
    },
    onDrop: (payload) => drop(surfaceKeyFromTabDragPayload(payload)),
  });
  return (
    <div
      ref={pointerDrop.ref}
      className={classNames(
        styles.tab,
        active && styles.active,
        styles.entering,
        pointerDrop.isOver && styles.dropTarget,
      )}
      role="presentation"
      style={{ minWidth: minimumWidth }}
      data-pane-tab-key={tab.key}
      onPointerDownCapture={(event) => {
        if (event.button !== 1 || href) return;
        event.preventDefault();
        event.stopPropagation();
      }}
      onMouseDownCapture={(event) => {
        if (event.button !== 1 || href) return;
        event.preventDefault();
        event.stopPropagation();
      }}
      onAuxClick={(event) => {
        if (event.button !== 1 || href) return;
        event.preventDefault();
        event.stopPropagation();
        if (!tab.navigation || tab.navigation.onClose) close(tab);
      }}
      onDragOver={(event) => {
        if (!canDrop) return;
        if (!hasSplittableTabDrag(event.dataTransfer)) return;
        event.preventDefault();
        event.stopPropagation();
        event.dataTransfer.dropEffect = "move";
      }}
      onDrop={(event) => {
        if (!canDrop) return;
        event.preventDefault();
        event.stopPropagation();
        drop(readTabDragSurfaceKey(event.dataTransfer));
      }}
    >
      <Tooltip>
        <Tooltip.Trigger asChild>
          <a
            href={href}
            role="tab"
            id={paneTabId(headerLeafId, tab.key)}
            aria-selected={active}
            aria-controls={
              tab.navigation
                ? tab.navigation.controlsId
                : hasOwner
                  ? panePanelId(leaf.id, tab.key)
                  : undefined
            }
            tabIndex={focusable ? 0 : -1}
            title={tab.display.tooltip ?? tab.title}
            className={styles.tabButton}
            draggable={!pointerEnabled && isSplittableSurface(surfaceKey)}
            onDragStart={(event) => {
              if (!isSplittableSurface(surfaceKey)) {
                event.preventDefault();
                return;
              }
              event.stopPropagation();
              event.dataTransfer.effectAllowed = "move";
              const dragKey = tab.navigation?.surfaceKey ?? tab.key;
              setTabDragData(event.dataTransfer, "surface", dragKey, dragKey);
            }}
            onPointerDown={(event) => {
              if (!pointerEnabled || !isSplittableSurface(surfaceKey)) return;
              cleanup.current?.();
              cleanup.current = beginPointerDragGesture(event, () => ({
                payload: {
                  type: "surface",
                  id: tab.navigation?.surfaceKey ?? tab.key,
                  surfaceKey: tab.navigation?.surfaceKey ?? tab.key,
                },
                label: tab.title,
              }));
            }}
            onClick={(event) => {
              if (href && !isPrimaryContentClick(event)) return;
              event.preventDefault();
              activate(tab.key);
            }}
            onKeyDown={(event) => {
              const index = keys.indexOf(tab.key);
              const next =
                event.key === "Home"
                  ? 0
                  : event.key === "End"
                    ? keys.length - 1
                    : event.key === "ArrowRight"
                      ? (index + 1) % keys.length
                      : event.key === "ArrowLeft"
                        ? (index + keys.length - 1) % keys.length
                        : null;
              if (next !== null) {
                event.preventDefault();
                activate(keys[next], true);
              }
            }}
          >
            <TabDisplayContent
              display={tab.display}
              titleClassName={styles.title}
            />
          </a>
        </Tooltip.Trigger>
        <Tooltip.Content
          side="bottom"
          align="start"
          maxWidth="min(24rem, calc(100vw - 24px))"
        >
          <TabHoverDetails display={tab.display} />
        </Tooltip.Content>
      </Tooltip>
      {(!tab.navigation || tab.navigation.onClose) && (
        <button
          type="button"
          className={styles.close}
          aria-label={`Close ${tab.title}`}
          onPointerDown={(event) => event.stopPropagation()}
          onClick={(event) => {
            event.stopPropagation();
            close(tab);
          }}
        >
          <Icon icon={X} size="sm" />
        </button>
      )}
    </div>
  );
}

export function PaneTabStrip({
  leaf,
  tabId,
  navigationEntries = [],
  selectedNavigationKey,
  onActivateLeaf,
  onFitChange,
  hidePaneActions = false,
  ownerBySurface,
  tablistLabel = "Pane tabs",
}: {
  leaf: LeafPane;
  tabId: SurfaceKey;
  navigationEntries?: PaneNavigationEntry[];
  selectedNavigationKey?: string | null;
  onActivateLeaf?: (surfaceKey: SurfaceKey) => void;
  hidePaneActions?: boolean;
  ownerBySurface?: Record<SurfaceKey, PaneTabOwner>;
  tablistLabel?: string;
  onFitChange?: PaneTabFitChange;
}) {
  const dispatch = useAppDispatch();
  const resolveDisplay = useTabDisplay();
  const header = useRef<HTMLDivElement>(null);
  const measures = useRef<HTMLDivElement>(null);
  const actions = useRef<HTMLDivElement>(null);
  const preview = useRef<HTMLDivElement>(null);
  const overflowMeasure = useRef<HTMLButtonElement>(null);
  const overflowTrigger = useRef<HTMLButtonElement>(null);
  const overflowSelection = useRef<SurfaceKey | null>(null);
  const [visibleKeys, setVisibleKeys] = useState(leaf.tabIds);
  const [minimums, setMinimums] = useState<Record<string, number>>({});
  const [focusKey, setFocusKey] = useState<SurfaceKey | null>(null);
  const activeKey =
    selectedNavigationKey === undefined
      ? leaf.activeTabId ?? leaf.tabIds[0]
      : selectedNavigationKey === null
        ? undefined
        : `nav:${selectedNavigationKey}`;
  const tabs: RenderTab[] = [
    ...leaf.tabIds.map((key): LocalTab => {
      const display = resolveDisplay(key);
      return {
        key,
        file: display.kind === "file",
        title: display.title,
        display,
      };
    }),
    ...navigationEntries.map((navigation) => {
      const display =
        navigation.display ?? resolveDisplay(navigation.surfaceKey);
      return {
        key: `nav:${navigation.key}`,
        file: false,
        title: display.title,
        display,
        navigation,
      };
    }),
  ];
  const keySignature = JSON.stringify(tabs.map((tab) => tab.key));
  const keys = useMemo<string[]>(
    () => JSON.parse(keySignature) as string[],
    [keySignature],
  );
  const renderedTabs = tabs.filter(
    (tab) => visibleKeys.includes(tab.key) || tab.key === activeKey,
  );
  const motion = useTabMotion<RenderTab>(
    renderedTabs.map((tab) => tab.key),
    `${tabId}:${leaf.id}`,
  );
  const labels = JSON.stringify(tabs.map((tab) => [tab.key, tab.display]));
  useLayoutEffect(() => {
    const measure = () => {
      if (!header.current || !measures.current) return;
      const width = header.current.clientWidth;
      const nodes = Array.from(measures.current.children);
      const widths = nodes.map((node) => node.getBoundingClientRect().width);
      onFitChange?.(
        header.current,
        widths.reduce((sum, value) => sum + value, 0),
      );
      if (!width) {
        setVisibleKeys(keys);
        return;
      }
      const floors = nodes.map((node, index) => {
        const title = node.querySelector<HTMLElement>(`.${styles.title}`);
        return title
          ? widths[index] -
              title.getBoundingClientRect().width +
              Math.min(
                title.getBoundingClientRect().width,
                parseFloat(getComputedStyle(title).minWidth) || 0,
              )
          : widths[index];
      });
      const remaining =
        width -
        (actions.current?.getBoundingClientRect().width ?? 0) -
        (preview.current?.getBoundingClientRect().width ?? 0);
      const selected = fitPaneTabs(
        floors,
        remaining,
        overflowMeasure.current?.getBoundingClientRect().width ?? 0,
        keys.indexOf(activeKey ?? ""),
      );
      const tabSpace = Math.max(
        0,
        remaining -
          (selected.length < keys.length
            ? overflowMeasure.current?.getBoundingClientRect().width ?? 0
            : 0),
      );
      const nextMinimums = Object.fromEntries(
        keys.map((key, index) => [
          key,
          selected.length === 1 && selected[0] === index
            ? Math.min(floors[index], tabSpace)
            : floors[index],
        ]),
      );
      setMinimums((previous) =>
        JSON.stringify(previous) === JSON.stringify(nextMinimums)
          ? previous
          : nextMinimums,
      );
      setVisibleKeys(keys.filter((_, index) => selected.includes(index)));
    };
    measure();
    const observer = new ResizeObserver(measure);
    if (header.current) observer.observe(header.current);
    if (preview.current) observer.observe(preview.current);
    const element = header.current;
    return () => {
      observer.disconnect();
      if (element) onFitChange?.(element, null);
    };
  }, [activeKey, labels, hidePaneActions, keys, onFitChange]);
  useLayoutEffect(() => {
    if (!focusKey) return;
    document.getElementById(paneTabId(leaf.id, focusKey))?.focus();
    setFocusKey(null);
  }, [focusKey, leaf.id, visibleKeys]);
  const activate = (surfaceKey: SurfaceKey, focus = false) => {
    const navigation = tabs.find((tab) => tab.key === surfaceKey)?.navigation;
    if (navigation) navigation.onActivate();
    else {
      const owner = ownerBySurface?.[surfaceKey] ?? { tabId, leaf };
      if (ownerBySurface && !(surfaceKey in ownerBySurface))
        dispatch(setActiveTab(surfaceKey));
      else
        dispatch(
          setPaneActive({
            tabId: owner.tabId,
            leafId: owner.leaf.id,
            surfaceKey,
          }),
        );
      onActivateLeaf?.(surfaceKey);
    }
    if (focus) setFocusKey(surfaceKey);
  };
  const close = (tab: RenderTab) => {
    motion.close(
      tab.key,
      () => {
        const remaining = keys.filter((key) => key !== tab.key);
        const successor =
          activeKey !== tab.key && activeKey
            ? activeKey
            : remaining[Math.min(keys.indexOf(tab.key), remaining.length - 1)];
        if (tab.navigation) tab.navigation.onClose?.();
        else {
          const owner = ownerBySurface?.[tab.key] ?? { tabId, leaf };
          if (ownerBySurface && !(tab.key in ownerBySurface))
            dispatch(closeTab(tab.key));
          else
            dispatch(
              closePaneTab({
                tabId: owner.tabId,
                leafId: owner.leaf.id,
                surfaceKey: tab.key,
              }),
            );
        }
        if (successor) setFocusKey(successor);
        else restorePaneTabFocus(!!ownerBySurface);
      },
      tab,
    );
  };
  const hidden = tabs.filter((tab) => !visibleKeys.includes(tab.key));
  const split = (dir: "row" | "col") => {
    dispatch(focusPane({ tabId, leafId: leaf.id }));
    dispatch(splitTab({ tabId, dir }));
  };
  return (
    <div
      ref={header}
      className={styles.header}
      aria-label={
        ownerBySurface ? "Workspace tab controls" : "Workspace pane controls"
      }
    >
      <div
        ref={motion.list}
        role="tablist"
        aria-label={tablistLabel}
        className={styles.tabs}
      >
        {renderedTabs.map((tab) => (
          <LocalTabButton
            key={tab.key}
            minimumWidth={minimums[tab.key]}
            tab={tab}
            leaf={ownerBySurface?.[tab.key]?.leaf ?? leaf}
            tabId={ownerBySurface?.[tab.key]?.tabId ?? tabId}
            headerLeafId={leaf.id}
            hasOwner={!ownerBySurface || tab.key in ownerBySurface}
            active={activeKey === tab.key}
            focusable={
              activeKey === tab.key ||
              (!activeKey &&
                tabs.find((item) => visibleKeys.includes(item.key))?.key ===
                  tab.key)
            }
            activate={activate}
            keys={keys}
            close={close}
          />
        ))}
        {motion.ghosts.map((ghost) => (
          <div
            key={`exit:${ghost.key}`}
            data-pane-tab-ghost={ghost.key}
            aria-hidden="true"
            className={classNames(styles.tab, styles.ghost)}
            style={{
              left: ghost.left,
              top: ghost.top,
              width: ghost.width,
              height: ghost.height,
            }}
          >
            <span className={styles.tabButton}>
              <TabDisplayContent
                display={ghost.content.display}
                titleClassName={styles.title}
              />
            </span>
            <span className={styles.close}>
              <Icon icon={X} size="sm" />
            </span>
          </div>
        ))}
      </div>
      {hidden.length > 0 && (
        <Menu>
          <Menu.Trigger asChild>
            <button
              type="button"
              ref={overflowTrigger}
              data-pane-tab-overflow="true"
              className={styles.overflow}
              aria-label={`${hidden.length} more pane tabs`}
            >
              +{hidden.length}
            </button>
          </Menu.Trigger>
          <Menu.Content
            onCloseAutoFocus={(event) => {
              const selected = overflowSelection.current;
              overflowSelection.current = null;
              if (!selected) return;
              // Selection hands focus to the newly visible tab, not the trigger.
              event.preventDefault();
              overflowTrigger.current?.focus();
              activate(selected, true);
            }}
          >
            {hidden.map((tab) => (
              <Menu.Item
                title={tab.display.tooltip ?? tab.title}
                key={tab.key}
                onSelect={() => {
                  overflowSelection.current = tab.key;
                }}
              >
                <TabDisplayContent
                  display={tab.display}
                  titleClassName={styles.title}
                />
              </Menu.Item>
            ))}
          </Menu.Content>
        </Menu>
      )}
      {!hidePaneActions && (
        <div ref={preview} className={styles.previewActions}>
          <PaneHeaderActions />
        </div>
      )}
      {!hidePaneActions && (
        <div ref={actions} className={styles.actions}>
          <Menu>
            <Menu.Trigger asChild>
              <IconButton
                icon={Ellipsis}
                size="sm"
                variant="plain"
                aria-label="Pane actions"
              />
            </Menu.Trigger>
            <Menu.Content align="end">
              <Menu.Item disabled={!activeKey} onSelect={() => split("row")}>
                <Icon icon={Columns3} size="sm" />
                Split Right
              </Menu.Item>
              <Menu.Item disabled={!activeKey} onSelect={() => split("col")}>
                <Icon icon={Rows3} size="sm" />
                Split Down
              </Menu.Item>
              <Menu.Separator />
              <Menu.Item
                disabled={!activeKey}
                onSelect={() => {
                  if (!activeKey) return;
                  const tab = tabs.find((item) => item.key === activeKey);
                  if (tab) close(tab);
                }}
              >
                Close tab
              </Menu.Item>
              <Menu.Item
                onSelect={() => {
                  dispatch(closePane({ tabId, leafId: leaf.id }));
                  restorePaneTabFocus(!!ownerBySurface);
                }}
              >
                Close Pane
              </Menu.Item>
            </Menu.Content>
          </Menu>
        </div>
      )}
      <div className={styles.measure} aria-hidden="true" ref={measures}>
        {tabs.map((tab) => (
          <span key={tab.key} className={styles.tab}>
            <span className={styles.tabButton}>
              <TabDisplayContent
                display={tab.display}
                titleClassName={styles.title}
              />
            </span>
            {(!tab.navigation || tab.navigation.onClose) && (
              <span className={styles.close}>
                <Icon icon={X} size="sm" />
              </span>
            )}
          </span>
        ))}
      </div>
      <button
        ref={overflowMeasure}
        className={classNames(styles.measure, styles.overflow)}
        aria-hidden="true"
        tabIndex={-1}
      >
        +{tabs.length}
      </button>
    </div>
  );
}
