import classNames from "classnames";
import { ChevronDown } from "lucide-react";
import {
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";

import { IconButton, StatusDot } from "../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import type { ExecStatus } from "../../../services/refact/exec";
import {
  selectTerminalSessions,
  type TerminalSessionMetadata,
} from "../TerminalPanel";
import {
  clampDrawerHeight,
  selectWorkspaceDrawer,
  setDrawerHeight,
  setDrawerOpen,
} from "../workspaceSlice";
import styles from "./Drawer.module.css";

type DrawerStyle = CSSProperties & {
  "--workspace-drawer-h": string;
};

type DrawerProps = {
  children: ReactNode;
};

function sessionStatus(status: ExecStatus): "running" | "error" | "idle" {
  if (status === "running" || status === "starting") return "running";
  if (status === "failed" || status === "timed_out") return "error";
  return "idle";
}

function sessionStatusLabel(session: TerminalSessionMetadata): string {
  return `${session.title}: ${session.status.replace("_", " ")}`;
}

export function Drawer({ children }: DrawerProps) {
  const dispatch = useAppDispatch();
  const drawer = useAppSelector(selectWorkspaceDrawer);
  const sessions = useAppSelector(selectTerminalSessions);
  const drawerRef = useRef<HTMLElement>(null);
  const liveHeightRef = useRef(drawer.height);
  const dragCleanupRef = useRef<(() => void) | null>(null);
  const [dragging, setDragging] = useState(false);
  const renderedHeight = clampDrawerHeight(
    drawer.height,
    typeof window === "undefined" ? drawer.height / 0.6 : window.innerHeight,
  );
  liveHeightRef.current = renderedHeight;

  const setOpen = useCallback(
    (open: boolean) => dispatch(setDrawerOpen(open)),
    [dispatch],
  );

  const handleResizePointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      const drawerElement = drawerRef.current;
      if (!drawerElement) return;

      event.preventDefault();
      dragCleanupRef.current?.();
      const startY = event.clientY;
      const startHeight = drawerElement.getBoundingClientRect().height;
      setDragging(true);
      document.body.style.cursor = "row-resize";
      document.body.style.userSelect = "none";

      const handlePointerMove = (moveEvent: PointerEvent) => {
        const next = clampDrawerHeight(
          startHeight + startY - moveEvent.clientY,
          window.innerHeight,
        );
        liveHeightRef.current = next;
        drawerElement.style.setProperty("--workspace-drawer-h", `${next}px`);
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
        dispatch(setDrawerHeight(liveHeightRef.current));
      };

      dragCleanupRef.current = detach;
      window.addEventListener("pointermove", handlePointerMove);
      window.addEventListener("pointerup", handlePointerUp);
      window.addEventListener("pointercancel", handlePointerUp);
    },
    [dispatch],
  );

  useEffect(() => () => dragCleanupRef.current?.(), []);

  return (
    <section
      ref={drawerRef}
      className={classNames(styles.drawer, !drawer.open && styles.collapsed)}
      aria-label="Terminal drawer"
      style={{ "--workspace-drawer-h": `${renderedHeight}px` } as DrawerStyle}
    >
      {drawer.open ? (
        <>
          <div
            aria-label="Resize terminal drawer"
            aria-orientation="horizontal"
            className={styles.splitter}
            data-dragging={dragging || undefined}
            onPointerDown={handleResizePointerDown}
            role="separator"
          >
            <div className={styles.splitterHandle} />
          </div>
          <div className={styles.openHeader}>
            <span className={styles.label}>Terminal</span>
            <span className={styles.count}>{sessions.length}</span>
            <div
              className={styles.statuses}
              aria-label="Terminal session status"
            >
              {sessions.map((session) => (
                <StatusDot
                  key={session.process_id}
                  aria-label={sessionStatusLabel(session)}
                  status={sessionStatus(session.status)}
                />
              ))}
            </div>
            <IconButton
              aria-label="Collapse terminal drawer"
              icon={ChevronDown}
              onClick={() => setOpen(false)}
              size="sm"
              variant="plain"
            />
          </div>
        </>
      ) : (
        <button
          type="button"
          className={styles.collapsedStrip}
          onClick={() => setOpen(true)}
          aria-label={`Expand terminal drawer, ${sessions.length} sessions`}
        >
          <span className={styles.label}>Terminal</span>
          <span className={styles.count}>{sessions.length}</span>
          <span
            className={styles.statuses}
            aria-label="Terminal session status"
          >
            {sessions.map((session) => (
              <StatusDot
                key={session.process_id}
                aria-label={sessionStatusLabel(session)}
                status={sessionStatus(session.status)}
              />
            ))}
          </span>
        </button>
      )}
      <div className={styles.content} aria-hidden={!drawer.open}>
        {children}
      </div>
    </section>
  );
}
