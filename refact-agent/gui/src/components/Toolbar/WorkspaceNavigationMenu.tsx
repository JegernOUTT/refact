import {
  CheckSquare,
  FileDiff,
  GitBranch,
  Palette,
  PanelsTopLeft,
  Users,
} from "lucide-react";

import { push } from "../../features/Pages/pagesSlice";
import { selectOpenTasksFromRoot } from "../../features/Tasks/tasksSlice";
import {
  activateWorkspaceLayout,
  selectTabs,
  setActiveTab,
} from "../../features/Workspace/workspaceSlice";
import {
  gitDiffDescriptorFromSurfaceKey,
  isSplittableSurface,
  parseSurfaceKey,
  type SurfaceKey,
} from "../../features/Workspace/surfaceKey";
import {
  gitDiffTitle,
  gitDiffTooltip,
  pathBasename,
} from "../../features/Workspace/GitPanel/diff/gitDiffDescriptor";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useAppSelector } from "../../hooks/useAppSelector";
import { Icon, IconButton, Menu, Tooltip } from "../ui";
import styles from "./WorkspaceNavigationMenu.module.css";

function surfaceDisplay(surfaceKey: SurfaceKey) {
  const parsed = parseSurfaceKey(surfaceKey);
  if (parsed.kind === "git") {
    return { title: "Git", tooltip: "Git", icon: GitBranch };
  }
  if (parsed.kind === "gitdiff") {
    const descriptor = gitDiffDescriptorFromSurfaceKey(surfaceKey);
    return {
      title: descriptor ? gitDiffTitle(descriptor) : "Diff",
      tooltip: descriptor ? gitDiffTooltip(descriptor) : "Diff",
      icon: FileDiff,
    };
  }
  if (parsed.kind === "design") {
    return {
      title:
        parsed.id === "main" ? "Design" : `Design · ${pathBasename(parsed.id)}`,
      tooltip: parsed.id === "main" ? "Design" : parsed.id,
      icon: Palette,
    };
  }
  return null;
}

export function WorkspaceNavigationMenu() {
  const dispatch = useAppDispatch();
  const tabs = useAppSelector(selectTabs);
  const openTasks = useAppSelector(selectOpenTasksFromRoot);
  const editorKey = tabs.find(isSplittableSurface);
  const surfaces = tabs.flatMap((key) => {
    const display = surfaceDisplay(key);
    return display ? [{ key, ...display }] : [];
  });

  const openWorkspaceView = (key: SurfaceKey) => {
    dispatch(setActiveTab(key));
    dispatch(push({ name: "chat" }));
  };

  return (
    <Menu>
      <Tooltip>
        <Tooltip.Trigger asChild>
          <Menu.Trigger asChild>
            <IconButton
              aria-label="Open views"
              className={styles.trigger}
              icon={PanelsTopLeft}
              size="sm"
              variant="plain"
            />
          </Menu.Trigger>
        </Tooltip.Trigger>
        <Tooltip.Content side="bottom">Open views</Tooltip.Content>
      </Tooltip>
      <Menu.Content align="start">
        {editorKey && (
          <Menu.Item
            onSelect={() => {
              dispatch(activateWorkspaceLayout());
              dispatch(push({ name: "chat" }));
            }}
          >
            <Icon icon={PanelsTopLeft} size="sm" /> Editor
          </Menu.Item>
        )}
        {surfaces.map((surface) => (
          <Menu.Item
            key={surface.key}
            title={surface.tooltip}
            onSelect={() => openWorkspaceView(surface.key)}
          >
            <Icon icon={surface.icon} size="sm" />
            <span className={styles.label}>{surface.title}</span>
          </Menu.Item>
        ))}
        {(Boolean(editorKey) || surfaces.length > 0) && <Menu.Separator />}
        {openTasks.map((task) => (
          <Menu.Item
            key={task.id}
            title={task.name}
            onSelect={() =>
              dispatch(push({ name: "task workspace", taskId: task.id }))
            }
          >
            <Icon icon={CheckSquare} size="sm" />
            <span className={styles.label}>{task.name}</span>
          </Menu.Item>
        ))}
        <Menu.Item onSelect={() => dispatch(push({ name: "buddy" }))}>
          <Icon icon={Users} size="sm" /> Buddy
        </Menu.Item>
      </Menu.Content>
    </Menu>
  );
}
