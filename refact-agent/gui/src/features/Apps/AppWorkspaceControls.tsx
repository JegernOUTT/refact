import { Button, Text } from "@radix-ui/themes";
import { AlertTriangle, Info } from "lucide-react";
import { Icon } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectFocusedWorkspaceSurfaceKey,
  selectWorkspaceFocusedChatId,
} from "../Workspace/workspaceSlice";
import { retryAppWorkspaceSave } from "./appsSlice";
import styles from "./AppWorkspaceControls.module.css";

export function AppWorkspaceControls() {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector((state) => {
    const surface = selectFocusedWorkspaceSurfaceKey(state);
    return (
      selectWorkspaceFocusedChatId(state) ??
      (surface ? state.apps.views[surface]?.chatId : undefined)
    );
  });
  const status = useAppSelector((state) =>
    chatId ? state.apps.saveStatus?.[chatId] : undefined,
  );
  if (!status) return null;
  const message =
    status === "saving"
      ? "Saving app layout…"
      : status === "saved"
        ? "App layout saved automatically."
        : status === "readonly"
          ? "This app workspace version is read-only."
          : "App layout could not be saved. Your local layout is preserved.";
  const isNotice = status === "error" || status === "readonly";
  return (
    <div
      className={isNotice ? styles.notice : styles.quietStatus}
      data-status={status}
      role="status"
      aria-live="polite"
      aria-atomic="true"
      aria-label="App layout"
    >
      <div className={styles.copy}>
        {isNotice && (
          <Icon
            icon={status === "error" ? AlertTriangle : Info}
            tone={status === "error" ? "warning" : "muted"}
            className={styles.icon}
          />
        )}
        <Text size="1" className={styles.message}>
          {message}
        </Text>
      </div>
      {status === "error" && chatId && (
        <Button
          size="1"
          variant="soft"
          className={styles.retry}
          onClick={() => dispatch(retryAppWorkspaceSave(chatId))}
        >
          Retry save
        </Button>
      )}
    </div>
  );
}
