import { useEffect } from "react";

import { useAppDispatch, useAppSelector } from "../../hooks";
import { selectCapabilities, selectHost } from "../Config/configSlice";
import {
  setTerminalWorkbenchOpen,
  toggleTerminalWorkbench,
} from "./TerminalPanel/terminalSlice";
import {
  selectFocusedWorkspaceChatId,
  selectPanelsForced,
  selectWorkspaceDock,
  setDockOpen,
  setDockSection,
  toggleDock,
  type WorkspaceDockSection,
} from "./workspaceSlice";
import { resolveWorkspaceDockAvailability } from "./workspaceAvailability";

function ownsWorkspaceShortcut(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  if (target instanceof HTMLElement && target.isContentEditable) return true;
  return Boolean(
    target.closest(
      'input, textarea, select, [contenteditable]:not([contenteditable="false"]), .xterm',
    ),
  );
}

export function useWorkspaceShortcuts() {
  const dispatch = useAppDispatch();
  const host = useAppSelector(selectHost);
  const capabilities = useAppSelector(selectCapabilities);
  const panelsForced = useAppSelector(selectPanelsForced);
  const dock = useAppSelector(selectWorkspaceDock);
  const focusedChatId = useAppSelector(selectFocusedWorkspaceChatId);

  useEffect(() => {
    if (host !== "web") return;

    const {
      dock: dockAvailable,
      files: filesAvailable,
      git: gitAvailable,
      tasks: tasksAvailable,
      terminal: terminalAvailable,
    } = resolveWorkspaceDockAvailability(capabilities, panelsForced);

    const handleKeyDown = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.repeat ||
        event.isComposing ||
        event.altKey ||
        event.shiftKey ||
        !(event.ctrlKey || event.metaKey) ||
        ownsWorkspaceShortcut(event.target)
      ) {
        return;
      }

      const key = event.key.toLowerCase();
      if (key === "b" && dockAvailable) {
        event.preventDefault();
        dispatch(toggleDock());
        return;
      }
      if (key === "j" && terminalAvailable && focusedChatId) {
        event.preventDefault();
        if (dock.open) {
          dispatch(toggleTerminalWorkbench({ chatId: focusedChatId }));
        } else {
          dispatch(setDockOpen(true));
          dispatch(
            setTerminalWorkbenchOpen({ chatId: focusedChatId, open: true }),
          );
        }
        return;
      }

      let section: WorkspaceDockSection;
      if (key === "1") section = "files";
      else if (key === "2") section = "git";
      else if (key === "3") section = "tasks";
      else return;
      const sectionAvailable =
        (section === "files" && filesAvailable) ||
        (section === "git" && gitAvailable) ||
        (section === "tasks" && tasksAvailable);
      if (!sectionAvailable) return;
      event.preventDefault();
      dispatch(setDockSection(section));
      dispatch(setDockOpen(true));
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [capabilities, dispatch, dock.open, focusedChatId, host, panelsForced]);
}
