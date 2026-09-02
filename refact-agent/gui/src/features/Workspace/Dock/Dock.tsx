import classNames from "classnames";
import { X } from "lucide-react";
import {
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { IconButton, Sheet, useMediaQuery } from "../../../components/ui";
import {
  COLLAPSE_ANIMATION_MS,
  useDelayedUnmount,
} from "../../../components/shared/useDelayedUnmount";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import { AgentsSection } from "../../AgentsPanel";
import { switchToThread } from "../../Chat/Thread";
import { selectCapabilities } from "../../Config/configSlice";
import { FilesPanel } from "../FilesPanel";
import { GitDock } from "../GitPanel";
import {
  normalizeDockWidth,
  selectFocusedWorkspaceChatId,
  selectPanelsForced,
  selectWorkspaceDock,
  setDockOpen,
  setDockSection,
  setDockWidth,
  type WorkspaceDockSection,
} from "../workspaceSlice";
import styles from "./Dock.module.css";
import { TasksSection } from "./TasksSection";

const narrowQuery = "(max-width: 767px)";

type DockStyle = CSSProperties & {
  "--workspace-dock-w": string;
};

export function Dock() {
  const dispatch = useAppDispatch();
  const capabilities = useAppSelector(selectCapabilities);
  const panelsForced = useAppSelector(selectPanelsForced);
  const dock = useAppSelector(selectWorkspaceDock);
  const focusedChatId = useAppSelector(selectFocusedWorkspaceChatId);
  const isNarrow = useMediaQuery(narrowQuery);
  const dockRef = useRef<HTMLElement>(null);
  const liveWidthRef = useRef(dock.width);
  const dragCleanupRef = useRef<(() => void) | null>(null);
  const [dragging, setDragging] = useState(false);
  const { shouldRender, isAnimatingOpen } = useDelayedUnmount(
    dock.open,
    COLLAPSE_ANIMATION_MS,
  );
  const availableSections = useMemo<WorkspaceDockSection[]>(() => {
    const result: WorkspaceDockSection[] = [];
    if (capabilities.filesPanel || panelsForced) result.push("files");
    if (capabilities.gitPanel || panelsForced) result.push("git");
    result.push("agents", "tasks");
    return result;
  }, [capabilities.filesPanel, capabilities.gitPanel, panelsForced]);
  const activeSection = availableSections.includes(dock.section)
    ? dock.section
    : availableSections[0];

  useEffect(() => {
    if (activeSection !== dock.section) {
      dispatch(setDockSection(activeSection));
    }
  }, [activeSection, dispatch, dock.section]);

  const handleAgentNavigation = useCallback(
    (childChatId: string) => {
      dispatch(switchToThread({ id: childChatId }));
    },
    [dispatch],
  );

  const handleResizePointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      const dockElement = dockRef.current;
      if (!dockElement) return;

      event.preventDefault();
      dragCleanupRef.current?.();
      const startX = event.clientX;
      const startWidth = dockElement.getBoundingClientRect().width;
      setDragging(true);
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";

      const handlePointerMove = (moveEvent: PointerEvent) => {
        const next = normalizeDockWidth(
          startWidth + moveEvent.clientX - startX,
        );
        liveWidthRef.current = next;
        dockElement.style.setProperty("--workspace-dock-w", `${next}px`);
      };

      const detach = () => {
        dragCleanupRef.current = null;
        setDragging(false);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
        window.removeEventListener("pointermove", handlePointerMove);
        window.removeEventListener("pointerup", handlePointerUp);
        window.removeEventListener("pointercancel", handlePointerUp);
      };

      const handlePointerUp = () => {
        detach();
        dispatch(setDockWidth(liveWidthRef.current));
      };

      dragCleanupRef.current = detach;
      window.addEventListener("pointermove", handlePointerMove);
      window.addEventListener("pointerup", handlePointerUp);
      window.addEventListener("pointercancel", handlePointerUp);
    },
    [dispatch],
  );

  useEffect(() => () => dragCleanupRef.current?.(), []);

  const content = (withClose: boolean) => (
    <div className={styles.panelInner}>
      {withClose && (
        <div className={styles.sheetHeader}>
          <Sheet.Close asChild>
            <IconButton
              aria-label="Close workspace panel"
              className={styles.sheetClose}
              icon={X}
              size="sm"
              variant="ghost"
            />
          </Sheet.Close>
        </div>
      )}
      <div
        key={activeSection}
        className={classNames(styles.content, "rf-enter")}
        data-testid="workspace-dock-section"
        data-section={activeSection}
      >
        {activeSection === "files" ? <FilesPanel /> : null}
        {activeSection === "git" ? <GitDock /> : null}
        {activeSection === "agents" ? (
          <AgentsSection
            chatId={focusedChatId}
            onNavigate={handleAgentNavigation}
          />
        ) : null}
        {activeSection === "tasks" ? <TasksSection /> : null}
      </div>
    </div>
  );

  if (isNarrow) {
    return (
      <Sheet
        modal={false}
        open={dock.open}
        onOpenChange={(open) => dispatch(setDockOpen(open))}
      >
        <Sheet.Content
          className={classNames(styles.sheet, "rf-grow-in")}
          maxWidth="400px"
          scrollable={false}
          side="left"
        >
          <Sheet.Title className={styles.srOnly}>Workspace dock</Sheet.Title>
          <Sheet.Description className={styles.srOnly}>
            Browse workspace files and sections.
          </Sheet.Description>
          {content(true)}
        </Sheet.Content>
      </Sheet>
    );
  }

  if (!dock.open && !shouldRender) return null;

  return (
    <aside
      aria-label="Workspace dock"
      className={styles.dock}
      data-state={isAnimatingOpen ? "open" : "closed"}
      data-testid="workspace-dock"
      ref={dockRef}
      style={{ "--workspace-dock-w": `${dock.width}px` } as DockStyle}
    >
      {content(false)}
      <div
        aria-label="Resize workspace dock"
        aria-orientation="vertical"
        className={styles.splitter}
        data-dragging={dragging || undefined}
        onPointerDown={handleResizePointerDown}
        role="separator"
      >
        <div className={styles.splitterHandle} />
      </div>
    </aside>
  );
}
