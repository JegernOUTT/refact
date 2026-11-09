import { useEffect } from "react";
import { LayoutGrid } from "lucide-react";
import { listenAppEvents } from "./appStream";
import { selectCurrentThreadId } from "../Chat/Thread";
import { useAppsStream } from "./useAppsStream";
import { IconButton } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { isAppRecord, useGetAppsQuery } from "../../services/refact/apps";
import { hasUsableEngineEndpoint } from "../../services/refact/apiUrl";
import { openRequestedApp } from "./appWorkspacePersistence";
import {
  selectFocusedWorkspaceChatId,
  selectWorkspaceDock,
  setDockOpen,
  setDockSection,
} from "../Workspace/workspaceSlice";

export function AppRail({ className }: { className?: string }) {
  useAppsStream();
  const dispatch = useAppDispatch();
  const focusedChatId = useAppSelector(selectFocusedWorkspaceChatId);
  const currentChatId = useAppSelector(selectCurrentThreadId);
  const chatId = focusedChatId ?? currentChatId;
  const ready = useAppSelector((state) =>
    hasUsableEngineEndpoint(state.config),
  );
  const dock = useAppSelector(selectWorkspaceDock);
  const catalogOpen = dock.open && dock.section === "apps";
  const { data } = useGetAppsQuery(chatId, {
    skip: !ready,
    pollingInterval: 15000,
  });
  useEffect(
    () =>
      listenAppEvents((event) => {
        if (
          event.kind !== "view_request" ||
          !isAppRecord(event.data) ||
          event.data.method !== "open" ||
          typeof event.data.app_id !== "string"
        )
          return;
        const appId = event.data.app_id;
        const bindingId =
          typeof event.data.binding_id === "string"
            ? event.data.binding_id
            : undefined;
        const owner =
          typeof event.data.chat_id === "string"
            ? event.data.chat_id
            : undefined;
        if ((event.hostChatId ?? undefined) !== owner) return;
        dispatch(
          openRequestedApp({
            appId,
            chatId: owner,
            bindingId,
            name:
              data?.apps.find((app) => app.manifest.app_id === appId)?.manifest
                .name ?? "App",
          }),
        );
      }),
    [dispatch, data],
  );
  return (
    <IconButton
      aria-label="Apps"
      aria-controls="workspace-apps-panel"
      aria-expanded={catalogOpen}
      aria-pressed={catalogOpen}
      className={className}
      icon={LayoutGrid}
      size="sm"
      variant="ghost"
      onClick={() => {
        dispatch(setDockSection("apps"));
        dispatch(setDockOpen(!catalogOpen));
      }}
    />
  );
}
