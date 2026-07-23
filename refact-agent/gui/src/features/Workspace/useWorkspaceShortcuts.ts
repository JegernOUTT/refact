import { useEffect } from "react";

import { useAppDispatch, useAppSelector } from "../../hooks";
import { selectCapabilities, selectHost } from "../Config/configSlice";
import {
  setDockOpen,
  setDockSection,
  toggleDock,
  toggleDrawer,
  type WorkspaceDockSection,
} from "./workspaceSlice";

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

  useEffect(() => {
    if (host !== "web") return;

    const dockAvailable = capabilities.filesPanel || capabilities.gitPanel;

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
      if (key === "j" && capabilities.terminalPanel) {
        event.preventDefault();
        dispatch(toggleDrawer());
        return;
      }

      let section: WorkspaceDockSection;
      if (key === "1") section = "files";
      else if (key === "2") section = "git";
      else if (key === "3") section = "tasks";
      else return;
      const sectionAvailable =
        section === "tasks" ||
        (section === "files" && capabilities.filesPanel) ||
        (section === "git" && capabilities.gitPanel);
      if (!dockAvailable || !sectionAvailable) return;
      event.preventDefault();
      dispatch(setDockSection(section));
      dispatch(setDockOpen(true));
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [capabilities, dispatch, host]);
}
