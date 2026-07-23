import { Files, GitBranch, ListTodo } from "lucide-react";
import {
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import {
  Icon,
  SegmentedControl,
  Sheet,
  useMediaQuery,
} from "../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import { selectCapabilities } from "../../Config/configSlice";
import { FilesPanel } from "../FilesPanel";
import {
  normalizeDockWidth,
  selectWorkspaceDock,
  setDockOpen,
  setDockSection,
  setDockWidth,
  type WorkspaceDockSection,
} from "../workspaceSlice";
import styles from "./Dock.module.css";

const narrowQuery = "(max-width: 767px)";

type DockStyle = CSSProperties & {
  "--workspace-dock-w": string;
};

type DockOption = {
  value: WorkspaceDockSection;
  label: React.ReactNode;
};

export function Dock() {
  const dispatch = useAppDispatch();
  const capabilities = useAppSelector(selectCapabilities);
  const dock = useAppSelector(selectWorkspaceDock);
  const isNarrow = useMediaQuery(narrowQuery);
  const dockRef = useRef<HTMLElement>(null);
  const liveWidthRef = useRef(dock.width);
  const dragCleanupRef = useRef<(() => void) | null>(null);
  const [dragging, setDragging] = useState(false);
  const options = useMemo<DockOption[]>(() => {
    const result: DockOption[] = [];
    if (capabilities.filesPanel) {
      result.push({
        value: "files",
        label: (
          <>
            <Icon icon={Files} size="sm" />
            Files
          </>
        ),
      });
    }
    if (capabilities.gitPanel) {
      result.push({
        value: "git",
        label: (
          <>
            <Icon icon={GitBranch} size="sm" />
            Git
          </>
        ),
      });
    }
    result.push({
      value: "tasks",
      label: (
        <>
          <Icon icon={ListTodo} size="sm" />
          Tasks
        </>
      ),
    });
    return result;
  }, [capabilities.filesPanel, capabilities.gitPanel]);
  const activeSection = options.some((option) => option.value === dock.section)
    ? dock.section
    : options[0].value;

  useEffect(() => {
    if (activeSection !== dock.section) {
      dispatch(setDockSection(activeSection));
    }
  }, [activeSection, dispatch, dock.section]);

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

  const content = (
    <>
      <div className={styles.switcher}>
        <SegmentedControl
          aria-label="Workspace dock sections"
          name="workspace-dock-section"
          onValueChange={(value) =>
            dispatch(setDockSection(value as WorkspaceDockSection))
          }
          options={options}
          size="sm"
          value={activeSection}
        />
      </div>
      <div className={styles.content}>
        {activeSection === "files" ? <FilesPanel /> : null}
        {activeSection !== "files" ? (
          <div className={styles.placeholder}>{`${activeSection === "git" ? "Git" : "Tasks"} coming soon`}</div>
        ) : null}
      </div>
    </>
  );

  if (isNarrow) {
    return (
      <Sheet
        open={dock.open}
        onOpenChange={(open) => dispatch(setDockOpen(open))}
      >
        <Sheet.Content
          className={styles.sheet}
          maxWidth="400px"
          scrollable={false}
          side="left"
        >
          <Sheet.Title className={styles.srOnly}>Workspace dock</Sheet.Title>
          <Sheet.Description className={styles.srOnly}>
            Browse workspace files and sections.
          </Sheet.Description>
          {content}
        </Sheet.Content>
      </Sheet>
    );
  }

  if (!dock.open) return null;

  return (
    <aside
      aria-label="Workspace dock"
      className={styles.dock}
      data-testid="workspace-dock"
      ref={dockRef}
      style={{ "--workspace-dock-w": `${dock.width}px` } as DockStyle}
    >
      {content}
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
