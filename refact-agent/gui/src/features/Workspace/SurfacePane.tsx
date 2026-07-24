import classNames from "classnames";
import { MessageSquare } from "lucide-react";
import { Suspense, useCallback } from "react";

import { EmptyState, Spinner } from "../../components/ui";
import { useAppSelector, useConfig } from "../../hooks";
import { Chat } from "../Chat/Chat";
import { ChatThreadProvider } from "../Chat/Thread";
import { selectCapabilities } from "../Config/configSlice";
import { FileViewer } from "./FilesPanel";
import { GitPanel } from "./GitPanel";
import { isGitSurface, parseSurfaceKey, type SurfaceKey } from "./surfaceKey";
import { selectPanelsForced } from "./workspaceSlice";
import styles from "./SurfacePane.module.css";

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
          <FileViewer path={parsed.id} />
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
