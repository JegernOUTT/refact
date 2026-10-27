import classNames from "classnames";
import { MessageSquare } from "lucide-react";
import { lazy, Suspense, useCallback } from "react";

import { EmptyState, Spinner } from "../../components/ui";
import { useAppSelector, useConfig } from "../../hooks";
import { Chat } from "../Chat/Chat";
import { ChatThreadProvider } from "../Chat/Thread";
import { selectCapabilities } from "../Config/configSlice";
import { DesignSurface } from "../Design";
import { FileViewer } from "./FilesPanel";
import { GitPanel } from "./GitPanel";
import {
  gitDiffDescriptorFromSurfaceKey,
  isGitSurface,
  parseSurfaceKey,
  type SurfaceKey,
} from "./surfaceKey";
import { selectPanelsForced } from "./workspaceSlice";
import styles from "./SurfacePane.module.css";

const GitDiffViewer = lazy(() => import("./GitPanel/diff/GitDiffViewer"));

export type SurfacePaneProps = {
  surfaceKey?: SurfaceKey | null;
};

export function SurfacePane({ surfaceKey }: SurfacePaneProps) {
  const config = useConfig();
  const capabilities = useAppSelector(selectCapabilities);
  const panelsForced = useAppSelector(selectPanelsForced);
  const backFromChat = useCallback(() => undefined, []);

  if (!surfaceKey) {
    return (
      <div className={classNames(styles.placeholder, "rf-enter-rise")}>
        <EmptyState
          icon={MessageSquare}
          title="No surface selected"
          description="Open or drag a workspace tab into this pane."
          variant="full"
        />
      </div>
    );
  }

  try {
    const parsed = parseSurfaceKey(surfaceKey);

    if (parsed.kind === "chat") {
      return (
        <div className={styles.surfacePane} data-surface-key={surfaceKey}>
          <ChatThreadProvider chatId={parsed.id}>
            <Chat
              host={config.host}
              tabbed={config.tabbed}
              backFromChat={backFromChat}
              chatId={parsed.id}
            />
          </ChatThreadProvider>
        </div>
      );
    }

    if (parsed.kind === "file") {
      if (!capabilities.filesPanel && !panelsForced) return null;
      return (
        <div
          className={classNames(styles.surfacePane, styles.fileSurface)}
          data-surface-key={surfaceKey}
        >
          <FileViewer key={parsed.id} path={parsed.id} />
        </div>
      );
    }

    if (parsed.kind === "design") {
      return (
        <div
          className={classNames(styles.surfacePane, styles.panelSurface)}
          data-surface-key={surfaceKey}
        >
          <DesignSurface surfaceId={parsed.id} />
        </div>
      );
    }

    if (parsed.kind === "gitdiff") {
      if (!capabilities.gitPanel && !panelsForced) return null;
      const descriptor = gitDiffDescriptorFromSurfaceKey(surfaceKey);
      if (!descriptor) return null;
      return (
        <div
          className={classNames(styles.surfacePane, styles.panelSurface)}
          data-surface-key={surfaceKey}
        >
          <Suspense fallback={<Spinner label="Loading diff viewer" />}>
            <GitDiffViewer
              key={surfaceKey}
              descriptor={descriptor}
              surfaceKey={surfaceKey}
            />
          </Suspense>
        </div>
      );
    }

    if (isGitSurface(surfaceKey)) {
      if (!capabilities.gitPanel && !panelsForced) return null;
      return (
        <div
          className={classNames(styles.surfacePane, styles.panelSurface)}
          data-surface-key={surfaceKey}
        >
          <Suspense fallback={<Spinner label="Loading Git panel" />}>
            <GitPanel />
          </Suspense>
        </div>
      );
    }

    return null;
  } catch {
    return null;
  }
}
