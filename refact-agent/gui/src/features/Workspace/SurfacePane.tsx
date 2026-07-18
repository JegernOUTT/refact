import classNames from "classnames";
import { MessageSquare } from "lucide-react";
import { Suspense, useCallback } from "react";

import { EmptyState, Spinner } from "../../components/ui";
import { useAppSelector, useConfig } from "../../hooks";
import { Chat } from "../Chat/Chat";
import { ChatThreadProvider } from "../Chat/Thread";
import { selectCapabilities } from "../Config/configSlice";
import { PANEL_COMPONENTS } from "./panels/panelRegistry";
import {
  isPanelKind,
  panelCapabilityKey,
  parseSurfaceKey,
  type SurfaceKey,
} from "./surfaceKey";
import styles from "./SurfacePane.module.css";

export type SurfacePaneProps = {
  surfaceKey?: SurfaceKey | null;
};

export function SurfacePane({ surfaceKey }: SurfacePaneProps) {
  const config = useConfig();
  const capabilities = useAppSelector(selectCapabilities);
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

    if (isPanelKind(parsed.kind)) {
      if (!capabilities[panelCapabilityKey(parsed.kind)]) return null;
      const PanelComponent = PANEL_COMPONENTS[parsed.kind];
      return (
        <div
          className={classNames(styles.surfacePane, styles.panelSurface)}
          data-surface-key={surfaceKey}
        >
          <Suspense
            fallback={<Spinner label={`Loading ${parsed.kind} panel`} />}
          >
            <PanelComponent />
          </Suspense>
        </div>
      );
    }

    return null;
  } catch {
    return null;
  }
}
