import { Globe } from "lucide-react";
import { IconButton, Tooltip } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectWorkspaceDock,
  setDockOpen,
  setDockSection,
} from "../Workspace/workspaceSlice";

export function BrowserRail({ className }: { className?: string }) {
  const dispatch = useAppDispatch();
  const dock = useAppSelector(selectWorkspaceDock);
  const open = dock.open && dock.section === "browser";
  return (
    <Tooltip content="Browser">
      <IconButton
        aria-label="Browser"
        aria-controls="workspace-browser-panel"
        aria-expanded={open}
        aria-pressed={open}
        className={className}
        icon={Globe}
        size="sm"
        variant="ghost"
        onClick={() => {
          dispatch(setDockSection("browser"));
          dispatch(setDockOpen(!open));
        }}
      />
    </Tooltip>
  );
}
