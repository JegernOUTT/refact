import { ArrowLeft, ArrowRight, Globe, RotateCw } from "lucide-react";
import { useEffect, useState } from "react";
import { Button, FieldText, Icon, IconButton } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  browserApi,
  isProjectBrowserTabGone,
  projectBrowserErrorMessage,
  type ProjectBrowserNavigateRequest,
} from "../../services/refact/browser";
import { usePaneSurfaceActive } from "../Workspace/PaneHeaderContext";
import { makeBrowserTabSurfaceKey } from "../Workspace/surfaceKey";
import { closeTab, selectBrowserViewOwners } from "../Workspace/workspaceSlice";
import { BrowserTabFrame } from "./BrowserTabFrame";
import {
  selectBrowserShowInProgress,
  selectBrowserTabAwaitingStatus,
} from "./browserViews";
import { useBrowserTabFrame } from "./useBrowserTabFrame";
import { useProjectBrowserStatus } from "./useProjectBrowserStatus";
import { useProjectBrowserTabActions } from "./useProjectBrowserTabActions";
import styles from "./ProjectBrowserTabSurface.module.css";

const NAVIGATION_OPS = [
  ["back", "Back", ArrowLeft],
  ["forward", "Forward", ArrowRight],
  ["reload", "Reload", RotateCw],
] as const;

export function ProjectBrowserTabSurface({ tabId }: { tabId: string }) {
  const dispatch = useAppDispatch();
  const active = usePaneSurfaceActive();
  const { status, error } = useProjectBrowserStatus();
  const awaitingStatus = useAppSelector((state) =>
    selectBrowserTabAwaitingStatus(state, tabId),
  );
  const ownerChatId = useAppSelector(
    (state) => selectBrowserViewOwners(state)[tabId] as string | undefined,
  );
  const showing = useAppSelector((state) =>
    ownerChatId ? selectBrowserShowInProgress(state, ownerChatId) : false,
  );
  const actions = useProjectBrowserTabActions();
  const [navigate] = browserApi.useProjectBrowserNavigateMutation();
  const [address, setAddress] = useState<string>();
  const [busy, setBusy] = useState(false);
  const view = useBrowserTabFrame(tabId, status, active, awaitingStatus, busy);
  const [commandError, setCommandError] = useState<string>();
  const tab = status?.tabs.find((item) => item.tab_id === tabId);
  const live = status?.connected === true && Boolean(tab) && !view.closed;
  const disabled = busy || status?.busy === true;
  const currentUrl = view.frame?.url ?? tab?.url ?? "";

  const run = async (request: ProjectBrowserNavigateRequest) => {
    if (disabled) return;
    setAddress(undefined);
    setBusy(true);
    setCommandError(undefined);
    try {
      await view.invalidate();
      await navigate(request).unwrap();
    } catch (cause) {
      if (isProjectBrowserTabGone(cause)) view.markClosed();
      else setCommandError(projectBrowserErrorMessage(cause));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    void dispatch(
      browserApi.endpoints.projectBrowserStatus.initiate(undefined, {
        forceRefetch: true,
        subscribe: false,
      }),
    );
  }, [dispatch, tabId]);

  const shownError = commandError ?? view.error ?? error;

  return (
    <section
      className={styles.surface}
      aria-label="Browser tab"
      data-browser-tab-id={tabId}
    >
      {live && (
        <form
          className={styles.navigation}
          aria-label="Browser navigation"
          onSubmit={(event) => {
            event.preventDefault();
            const url = (address ?? currentUrl).trim();
            if (url) void run({ url, tab_id: tabId });
          }}
        >
          {NAVIGATION_OPS.map(([op, label, Icon]) => (
            <IconButton
              key={op}
              type="button"
              size="sm"
              variant="ghost"
              aria-label={label}
              title={label}
              icon={Icon}
              disabled={disabled}
              onClick={() => void run({ op, tab_id: tabId })}
            />
          ))}
          <FieldText
            className={styles.address}
            aria-label="Address"
            placeholder="Enter a URL"
            value={address ?? currentUrl}
            disabled={disabled}
            onChange={setAddress}
            onKeyDown={(event) => {
              if (event.key === "Escape") setAddress(undefined);
            }}
          />
          <Button type="submit" size="sm" variant="soft" disabled={disabled}>
            Go
          </Button>
        </form>
      )}
      {shownError && !view.closed && (
        <div className={styles.error} role="alert">
          {shownError}
        </div>
      )}
      <div className={styles.content} ref={view.content}>
        {view.closed ? (
          <div className={styles.message}>
            <Icon icon={Globe} size="lg" tone="muted" />
            <h2>This browser tab is closed</h2>
            <p className={styles.muted}>
              Reopen a browser tab from the Browser rail.
            </p>
            {actions.error && <p role="alert">{actions.error}</p>}
            <div className={styles.actions}>
              {ownerChatId && (
                <Button
                  variant="primary"
                  loading={actions.pending}
                  disabled={actions.pending || showing}
                  onClick={() => void actions.showBrowserForChat(ownerChatId)}
                >
                  Open browser for this chat
                </Button>
              )}
              <Button
                variant="soft"
                onClick={() =>
                  dispatch(closeTab(makeBrowserTabSurfaceKey(tabId)))
                }
              >
                Close view
              </Button>
            </div>
          </div>
        ) : view.frame ? (
          <BrowserTabFrame
            frame={view.frame}
            disabled={disabled}
            onInput={view.sendInput}
          />
        ) : (
          <div className={styles.message}>
            <Icon icon={Globe} size="lg" tone="muted" />
            {!status ? (
              <p role="status">Checking project browser…</p>
            ) : (
              <p role="status">
                {busy || !status.connected
                  ? "Connecting to browser…"
                  : "Waiting for a browser frame…"}
              </p>
            )}
          </div>
        )}
      </div>
    </section>
  );
}
