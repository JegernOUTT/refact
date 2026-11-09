import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type RefObject,
} from "react";
import { ArrowLeft, ArrowUpRight, Info, RefreshCw, Trash2 } from "lucide-react";
import {
  Button,
  IconButton,
  Badge,
  Tooltip,
  useMediaQuery,
} from "../../components/ui";
import {
  useAppCommandMutation,
  isAppRecord,
  isAppManifest,
  type CatalogApp,
  useGetAppsQuery,
} from "../../services/refact/apps";
import { humanizeIdentifier } from "../../utils/displayNames";
import { AppIcon } from "./AppIcon";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { useInternalNavigation } from "../../hooks/useInternalNavigation";
import styles from "./AppsCatalog.module.css";
import { hasUsableEngineEndpoint } from "../../services/refact/apiUrl";
import { selectCurrentThreadId } from "../Chat/Thread";
import { openRailApp } from "./appsSlice";
import {
  selectFocusedWorkspaceChatId,
  setDockOpen,
} from "../Workspace/workspaceSlice";
import { WORKSPACE_NARROW_QUERY } from "../Workspace/workspaceBreakpoints";
import { HostAuthorization } from "../HostAuthorization/HostAuthorization";

type Props = {
  onNavigate: () => void;
  apps: CatalogApp[];
  chatId?: string;
  loading: boolean;
  unavailable: boolean;
  error: boolean;
  unauthorized?: boolean;
  onRetry: () => void;
  onOpen: (app: CatalogApp) => void;
};
function safeEndpoint(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  try {
    const url = new URL(value);
    return url.origin;
  } catch {
    return undefined;
  }
}
type Selection = { type: "details" | "remove"; app: CatalogApp };

function AppDetails({
  app,
  chatId,
  onClose,
}: {
  app: CatalogApp;
  chatId?: string;
  onClose: () => void;
}) {
  const navigation = useInternalNavigation({ scope: chatId ?? "" });
  const [command] = useAppCommandMutation();
  const [result, setResult] = useState<Record<string, unknown>>();
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let current = true;
    setResult(undefined);
    setFailed(false);
    void command({
      action: "details",
      app_id: app.manifest.app_id,
      hostChatId: chatId,
    })
      .unwrap()
      .then((value) => {
        if (!current) return;
        if (
          !isAppRecord(value) ||
          !isAppRecord(value.app) ||
          !isAppManifest(value.app.manifest) ||
          value.app.manifest.app_id !== app.manifest.app_id ||
          !Array.isArray(value.connectors) ||
          !Array.isArray(value.chats)
        ) {
          setFailed(true);
          return;
        }
        setResult(value);
      })
      .catch(() => {
        if (current) setFailed(true);
      });
    return () => {
      current = false;
    };
  }, [app.manifest.app_id, chatId, command, attempt]);
  if (failed)
    return (
      <div role="alert">
        Details could not be loaded.{" "}
        <Button onClick={() => setAttempt((value) => value + 1)}>
          Retry details
        </Button>
      </div>
    );
  if (!result || !isAppRecord(result.app))
    return <p role="status">Loading details…</p>;
  const record = result.app;
  const coverage = isAppRecord(result.coverage) ? result.coverage : {};
  const files = Array.isArray(record.files)
    ? record.files.filter((file): file is string => typeof file === "string")
    : [];
  const connectors = Array.isArray(result.connectors)
    ? result.connectors.filter(isAppRecord)
    : [];
  const chats = Array.isArray(result.chats)
    ? result.chats.filter(isAppRecord)
    : [];
  return (
    <div className={styles.details}>
      <dl className={styles.facts}>
        <dt>App ID</dt>
        <dd>{app.manifest.app_id}</dd>
        <dt>Draft revision</dt>
        <dd>
          {typeof record.revision === "string"
            ? record.revision
            : "Unavailable"}
        </dd>
        <dt>Published revision</dt>
        <dd>
          {typeof record.active_revision === "string"
            ? record.active_revision
            : "Not published"}
        </dd>
        <dt>Entrypoint</dt>
        <dd>
          {isAppManifest(record.manifest)
            ? record.manifest.entrypoint
            : "Unavailable"}
        </dd>
      </dl>
      <section aria-label="Files">
        <h3>Files</h3>
        {files.length ? (
          <ul>
            {files.map((file) => (
              <li key={file}>{file}</li>
            ))}
          </ul>
        ) : (
          <p>No files reported.</p>
        )}
      </section>
      <section aria-label="Connectors">
        <h3>Connectors</h3>
        {connectors.length ? (
          <ul>
            {connectors.map((connector, index) => {
              const source = isAppRecord(connector.source)
                ? connector.source
                : {};
              const runtime = isAppRecord(connector.runtime)
                ? connector.runtime
                : {};
              const driver = isAppRecord(source.driver) ? source.driver : {};
              return (
                <li key={index}>
                  <strong>
                    {typeof source.source_id === "string"
                      ? source.source_id
                      : "Unnamed connector"}
                  </strong>
                  {" — "}
                  {typeof driver.type === "string"
                    ? humanizeIdentifier(driver.type)
                    : "Unknown driver"}
                  {typeof source.channel === "string" && (
                    <span> · Channel: {source.channel}</span>
                  )}
                  {typeof driver.every_ms === "number" && (
                    <span> · Every {driver.every_ms} ms</span>
                  )}
                  {safeEndpoint(driver.url) && (
                    <span>
                      {" "}
                      · Endpoint: {safeEndpoint(driver.url)} (path and
                      credentials hidden)
                    </span>
                  )}
                  {" · "}
                  {typeof runtime.status === "string"
                    ? humanizeIdentifier(runtime.status)
                    : "No runtime status"}
                </li>
              );
            })}
          </ul>
        ) : (
          <p>No connectors declared.</p>
        )}
      </section>
      {navigation.pending && <p role="status">Opening linked chat…</p>}
      {navigation.error && <p role="alert">{navigation.error}</p>}
      <section aria-label="Connected chats">
        <h3>Connected chats</h3>
        {chats.length ? (
          <ul>
            {chats.map((chat, index) => (
              <li key={index}>
                {typeof chat.chat_id === "string" ? (
                  <Button
                    disabled={navigation.pending}
                    onClick={() => {
                      const id = String(chat.chat_id);
                      void navigation
                        .navigate(`refact://chat/${id}`, { chatId })
                        .then((opened) => {
                          if (opened) onClose();
                        });
                    }}
                  >
                    {typeof chat.title === "string" && chat.title
                      ? chat.title
                      : "Untitled chat"}
                  </Button>
                ) : (
                  "Untitled chat"
                )}
                {typeof chat.chat_id === "string" ? (
                  <span className={styles.muted}> — {chat.chat_id}</span>
                ) : null}
              </li>
            ))}
          </ul>
        ) : (
          <p>No connected chats reported.</p>
        )}
      </section>
      <p className={styles.muted}>
        {coverage.complete === true && coverage.truncated !== true
          ? "Project inventory complete."
          : "Project inventory partial or coverage unknown; some files, connectors or chats may be missing."}{" "}
        Runtime status is a snapshot; credentials, source payloads and host
        tokens are not displayed.
      </p>
    </div>
  );
}

function RemoveApp({
  app,
  chatId,
  onRemoved,
  onCancel,
  cancelRef,
}: {
  app: CatalogApp;
  chatId?: string;
  onRemoved: () => void;
  onCancel: () => void;
  cancelRef: RefObject<HTMLButtonElement>;
}) {
  const [command, { isLoading }] = useAppCommandMutation();
  const [failed, setFailed] = useState(false);
  const remove = async () => {
    setFailed(false);
    try {
      const result = await command({
        action: "remove",
        app_id: app.manifest.app_id,
        expected_revision: app.revision,
        hostChatId: chatId,
      }).unwrap();
      if (!isAppRecord(result) || result.removed !== true) {
        setFailed(true);
        return;
      }
      onRemoved();
    } catch {
      setFailed(true);
    }
  };
  return (
    <>
      <p>
        Remove <strong>{app.manifest.name}</strong>? This permanently deletes
        its app files and saved state. Existing chat bindings are preserved but
        become unavailable until detached. Chats are not deleted.
      </p>
      <p className={styles.muted}>
        Removal applies only to the draft revision shown when you selected
        Remove. If the app has changed, cancel and refresh the catalog before
        trying again.
      </p>
      {failed && (
        <p role="alert">
          Removal failed or could not be confirmed. The app has not been removed
          from this catalog. Retry, or cancel and refresh if its revision
          changed.
        </p>
      )}
      <div className={styles.actions}>
        <Button ref={cancelRef} disabled={isLoading} onClick={onCancel}>
          Cancel
        </Button>
        <Button
          variant="danger"
          loading={isLoading}
          onClick={() => {
            void remove();
          }}
        >
          {failed ? "Retry removal" : "Confirm removal"}
        </Button>
      </div>
    </>
  );
}

export function AppsCatalog({
  onNavigate,
  apps,
  chatId,
  loading,
  unavailable,
  error,
  unauthorized,
  onRetry,
  onOpen,
}: Props) {
  const [selection, setSelection] = useState<Selection>();
  const backRef = useRef<HTMLButtonElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const refreshRef = useRef<HTMLButtonElement>(null);
  const actionRefs = useRef(new Map<string, HTMLButtonElement>());
  const returnFocus = useRef<{ key: string; removedId?: string }>();
  const closeSelection = (removed = false) => {
    if (selection) {
      returnFocus.current = {
        key: `${selection.app.manifest.app_id}:${selection.type}`,
        removedId: removed ? selection.app.manifest.app_id : undefined,
      };
    }
    setSelection(undefined);
  };
  useLayoutEffect(() => {
    if (selection) {
      (selection.type === "remove" ? cancelRef : backRef).current?.focus();
    }
  }, [selection]);
  useLayoutEffect(() => {
    if (selection !== undefined || returnFocus.current === undefined) return;
    const { key, removedId } = returnFocus.current;
    const nextApp = apps.find((app) => app.manifest.app_id !== removedId);
    const origin = removedId ? undefined : actionRefs.current.get(key);
    const fallback = nextApp
      ? actionRefs.current.get(`${nextApp.manifest.app_id}:details`)
      : undefined;
    const target = origin ?? fallback ?? refreshRef.current;
    if (target && !target.disabled) {
      target.focus();
      returnFocus.current = undefined;
    }
  }, [selection, apps, loading]);
  return (
    <section
      id="workspace-apps-panel"
      aria-label="Apps"
      className={styles.catalog}
    >
      <header className={styles.header}>
        {selection && (
          <Tooltip content="Back to apps">
            <IconButton
              ref={backRef}
              icon={ArrowLeft}
              size="sm"
              aria-label="Back to apps"
              onClick={() => closeSelection()}
            />
          </Tooltip>
        )}
        <h2>Apps</h2>
        {!selection && !loading && !unavailable && !error && (
          <Badge size="xs" tone="muted" aria-label={`${apps.length} apps`}>
            {apps.length}
          </Badge>
        )}
        {!selection && !unavailable && (
          <Tooltip content="Refresh apps">
            <IconButton
              ref={refreshRef}
              className={styles.refresh}
              icon={RefreshCw}
              size="sm"
              aria-label="Refresh apps"
              disabled={loading}
              onClick={onRetry}
            />
          </Tooltip>
        )}
      </header>
      {selection && (
        <h3 className={styles.selectionHeading}>{`${
          selection.type === "details" ? "Details" : "Remove"
        } — ${selection.app.manifest.name}`}</h3>
      )}
      {selection ? (
        <div
          key={`${chatId ?? "standalone"}:${selection.type}:${
            selection.app.manifest.app_id
          }`}
        >
          {selection.type === "details" ? (
            <AppDetails
              app={selection.app}
              chatId={chatId}
              onClose={onNavigate}
            />
          ) : (
            <RemoveApp
              app={selection.app}
              chatId={chatId}
              cancelRef={cancelRef}
              onCancel={() => closeSelection()}
              onRemoved={() => closeSelection(true)}
            />
          )}
        </div>
      ) : (
        <>
          {unavailable ? (
            <p role="status">Connect to an engine to discover project apps.</p>
          ) : loading ? (
            <p role="status">Loading apps…</p>
          ) : unauthorized ? (
            <HostAuthorization resource="apps" />
          ) : error ? (
            <div role="alert">
              Apps could not be loaded. <Button onClick={onRetry}>Retry</Button>
            </div>
          ) : apps.length === 0 ? (
            <p>
              No apps yet. Ask your assistant to create a project app, then find
              it here.
            </p>
          ) : (
            <ul className={styles.grid} aria-label="Apps">
              {apps.map((app) => (
                <li
                  className={styles.card}
                  key={app.manifest.app_id}
                  aria-label={app.manifest.name}
                >
                  <div className={styles.cardHeading}>
                    <span className={styles.appIcon}>
                      <AppIcon
                        icon={app.manifest.icon}
                        artwork={app.icon_artwork}
                        className={styles.artwork}
                      />
                    </span>
                    <h3 title={app.manifest.name}>{app.manifest.name}</h3>
                  </div>
                  <Badge className={styles.status} size="xs" tone="muted">
                    {app.active_revision ? "Published" : "Draft"}
                  </Badge>
                  {app.manifest.description.trim() && (
                    <p
                      className={styles.description}
                      title={app.manifest.description}
                    >
                      {app.manifest.description}
                    </p>
                  )}
                  <div className={styles.tileActions}>
                    <Tooltip
                      content={
                        app.active_revision
                          ? `Open ${app.manifest.name}`
                          : "Publish this app before opening it"
                      }
                    >
                      <IconButton
                        className={styles.openAction}
                        icon={ArrowUpRight}
                        size="sm"
                        variant="soft"
                        aria-label={`Open ${app.manifest.name}`}
                        aria-disabled={!app.active_revision}
                        title={
                          !app.active_revision
                            ? "Publish this app before opening it"
                            : undefined
                        }
                        onClick={() => {
                          if (app.active_revision) onOpen(app);
                        }}
                      />
                    </Tooltip>
                    <Tooltip content={`Details for ${app.manifest.name}`}>
                      <IconButton
                        ref={(node) => {
                          const key = `${app.manifest.app_id}:details`;
                          if (node) actionRefs.current.set(key, node);
                          else actionRefs.current.delete(key);
                        }}
                        icon={Info}
                        size="sm"
                        aria-label={`Details for ${app.manifest.name}`}
                        onClick={() =>
                          setSelection({
                            type: "details",
                            app: structuredClone(app),
                          })
                        }
                      />
                    </Tooltip>
                    <Tooltip content={`Remove ${app.manifest.name}`}>
                      <IconButton
                        ref={(node) => {
                          const key = `${app.manifest.app_id}:remove`;
                          if (node) actionRefs.current.set(key, node);
                          else actionRefs.current.delete(key);
                        }}
                        icon={Trash2}
                        size="sm"
                        variant="danger"
                        aria-label={`Remove ${app.manifest.name}`}
                        onClick={() =>
                          setSelection({
                            type: "remove",
                            app: structuredClone(app),
                          })
                        }
                      />
                    </Tooltip>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </section>
  );
}

/** Dock content only; the rail owns the always-mounted stream listener. */
export function AppsDockPanel() {
  const dispatch = useAppDispatch();
  const focusedChatId = useAppSelector(selectFocusedWorkspaceChatId);
  const currentChatId = useAppSelector(selectCurrentThreadId);
  const chatId = focusedChatId ?? currentChatId;
  const ready = useAppSelector((state) =>
    hasUsableEngineEndpoint(state.config),
  );
  const isNarrow = useMediaQuery(WORKSPACE_NARROW_QUERY);
  const { data, isLoading, isError, error, refetch } = useGetAppsQuery(chatId, {
    skip: !ready,
    pollingInterval: 15000,
  });
  const onNavigate = () => {
    if (isNarrow) dispatch(setDockOpen(false));
  };
  return (
    <AppsCatalog
      key={chatId}
      apps={data?.apps ?? []}
      chatId={chatId}
      loading={isLoading}
      unavailable={!ready}
      error={isError}
      unauthorized={
        error !== undefined && "status" in error && error.status === 401
      }
      onRetry={() => {
        void refetch();
      }}
      onNavigate={onNavigate}
      onOpen={(app) => {
        dispatch(
          openRailApp({ appId: app.manifest.app_id, name: app.manifest.name }),
        );
        onNavigate();
      }}
    />
  );
}
