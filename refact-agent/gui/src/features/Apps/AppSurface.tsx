import { parseAppCaptureResult } from "./appCapture";
import { listenAppEvents } from "./appStream";
import { useEffect, useMemo, useRef, useState } from "react";
import { useAppearance } from "../../hooks/useAppearance";
import { Button, Flex, Text } from "@radix-ui/themes";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  isAppRecord,
  useAppCommandMutation,
  useGetAppDocumentQuery,
} from "../../services/refact/apps";
import type {
  AppStateScope,
  AppViewIdentity,
} from "../../services/refact/apps";
import { appContextStaged, canStageAppContext } from "./appsSlice";
import { buildAppDocument, APP_IFRAME_SANDBOX } from "./appDocument";
import { AppBridgeHost, createAppChannel } from "./appSdk";
import {
  readAppThemeTokens,
  sameAppThemeTokens,
  type AppThemeTokens,
} from "./appTheme";
import { v4 as uuidv4 } from "uuid";
import styles from "./AppSurface.module.css";
import { closeTab, setActiveTab } from "../Workspace/workspaceSlice";

type Props = {
  appId: string;
  bindingId?: string;
  chatId?: string;
  scopeChatId?: string;
  onFocus?: () => void;
  onClose?: () => void;
};
function isViewIdentity(value: unknown): value is AppViewIdentity {
  return (
    isAppRecord(value) &&
    typeof value.view_id === "string" &&
    typeof value.view_token === "string" &&
    typeof value.generation === "string" &&
    typeof value.revision === "string"
  );
}
export function AppSurface(props: Props) {
  const key = [
    props.appId,
    props.bindingId ?? "",
    props.chatId ?? "",
    props.scopeChatId ?? "",
  ].join("\u0000");
  return <AppSurfaceView key={key} {...props} />;
}
function AppSurfaceView({
  appId,
  bindingId,
  chatId,
  scopeChatId,
  onFocus,
  onClose,
}: Props) {
  const dispatch = useAppDispatch();
  const { appearance } = useAppearance();
  const theme = useRef(appearance);
  theme.current = appearance;
  const root = useRef<HTMLDivElement>(null);
  const tokens = useRef<AppThemeTokens | null>(null);
  const readTokens = () => readAppThemeTokens(root.current);
  const pending = useAppSelector((state) =>
    bindingId ? state.apps.pendingBindings?.[bindingId] === true : false,
  );
  const contexts = useAppSelector((state) =>
    chatId ? state.apps.stagedByChat[chatId] : undefined,
  );
  const staged = useRef(contexts);
  staged.current = contexts;
  const {
    data: document,
    error,
    isLoading,
    refetch,
  } = useGetAppDocumentQuery(
    { appId, bindingId, chatId: scopeChatId ?? chatId },
    { skip: pending },
  );
  const [command] = useAppCommandMutation();
  const iframe = useRef<HTMLIFrameElement>(null);
  const [identity, setIdentity] = useState<AppViewIdentity | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const apiKey = useAppSelector((state) => state.config.apiKey);
  const [notice, setNotice] = useState<{
    text: string;
    reconnect?: boolean;
  } | null>(null);
  const [registrationEpoch, setRegistrationEpoch] = useState(0);
  const channel = useMemo(createAppChannel, [registrationEpoch]);
  const stateRecord = useRef(document?.state);
  if (document && !stateRecord.current) stateRecord.current = document.state;
  const documentRef = useRef(document);
  documentRef.current = document;
  const publishedHtml = document?.html;
  const documentReady = document !== undefined;
  const bridgeRef = useRef<AppBridgeHost | null>(null);
  const controls = useRef({ onFocus, onClose });
  controls.current = { onFocus, onClose };
  useEffect(() => {
    const push = () => {
      const next = readTokens();
      if (tokens.current && sameAppThemeTokens(tokens.current, next)) return;
      tokens.current = next;
      bridgeRef.current?.notify("theme", {
        appearance: theme.current,
        tokens: next,
      });
    };
    tokens.current = null;
    push();
    const host = root.current?.closest("[data-appearance], .radix-themes");
    if (!host || typeof MutationObserver === "undefined") return;
    const observer = new MutationObserver(push);
    observer.observe(host, {
      attributes: true,
      attributeFilter: ["class", "style", "data-appearance", "data-host"],
    });
    return () => observer.disconnect();
  }, [appearance]);
  const source = useMemo(() => {
    if (publishedHtml === undefined || !identity) return undefined;
    tokens.current = readTokens();
    return buildAppDocument({
      html: publishedHtml,
      channel,
      generation: 1,
      appearance: theme.current,
      themeTokens: tokens.current,
      initialState: stateRecord.current?.value,
      title: appId,
    });
  }, [publishedHtml, identity, channel, appId]);
  useEffect(() => {
    if (pending) return;
    let disposed = false;
    let registered: AppViewIdentity | null = null;
    setIdentity(null);
    setFailure(null);
    setNotice(null);
    void command({
      action: "view_register",
      app_id: appId,
      chat_id: chatId,
      binding_id: bindingId,
      hostChatId: scopeChatId ?? chatId ?? null,
    })
      .unwrap()
      .then((response) => {
        if (!isViewIdentity(response))
          throw new Error("Invalid app view registration");
        registered = response;
        if (disposed)
          void command({
            action: "view_unregister",
            view_id: response.view_id,
            view_token: response.view_token,
            hostChatId: scopeChatId ?? chatId ?? null,
          });
        else setIdentity(response);
      })
      .catch(() => {
        if (!disposed) {
          setFailure("App view registration failed.");
        }
      });
    return () => {
      disposed = true;
      if (registered)
        void command({
          action: "view_unregister",
          view_id: registered.view_id,
          view_token: registered.view_token,
          hostChatId: scopeChatId ?? chatId ?? null,
        });
    };
  }, [
    appId,
    bindingId,
    chatId,
    scopeChatId,
    command,
    registrationEpoch,
    pending,
    apiKey,
  ]);
  useEffect(() => {
    if (!identity || !documentReady) return;
    let active = true;
    const credentials = {
      view_id: identity.view_id,
      view_token: identity.view_token,
      hostChatId: scopeChatId ?? chatId ?? null,
    };
    const scope: AppStateScope = bindingId
      ? { type: "binding", binding_id: bindingId }
      : { type: "standalone" };
    let stateRevision = stateRecord.current?.revision ?? 0;
    let appReady = false;
    let lastPresence = "";
    const beat = (force: boolean) => {
      if (!active) return;
      const frame = iframe.current;
      let visible = window.document.visibilityState !== "hidden";
      for (
        let node: HTMLElement | null = root.current;
        node;
        node = node.parentElement
      ) {
        const style = window.getComputedStyle(node);
        if (
          node.hidden ||
          style.display === "none" ||
          style.visibility === "hidden" ||
          style.visibility === "collapse"
        )
          visible = false;
      }
      const presence = {
        ready: appReady,
        visible,
        focused:
          visible && frame !== null && window.document.activeElement === frame,
      };
      const key = JSON.stringify(presence);
      if (!force && key === lastPresence) return;
      lastPresence = key;
      void command({ action: "view_heartbeat", ...credentials, ...presence })
        .unwrap()
        .then(() => {
          if (active) setNotice(null);
        })
        .catch(() => {
          lastPresence = "";
          if (active)
            setNotice({
              text: "App view connection lost. Reconnect to restore live features.",
              reconnect: true,
            });
        });
    };
    const sourceVersions = new Map<
      string,
      { generation: string; seq: number }
    >();
    const deliverSource = (value: unknown) => {
      if (
        !isAppRecord(value) ||
        value.app_id !== appId ||
        typeof value.source_id !== "string" ||
        typeof value.channel !== "string" ||
        typeof value.generation !== "string" ||
        typeof value.seq !== "number"
      )
        return;
      const previous = sourceVersions.get(value.source_id);
      if (
        previous?.generation === value.generation &&
        previous.seq >= value.seq
      )
        return;
      sourceVersions.set(value.source_id, {
        generation: value.generation,
        seq: value.seq,
      });
      bridge.notify("data", { name: value.channel, payload: value });
    };
    const refreshSources = () =>
      command({
        action: "source_list",
        app_id: appId,
        ...{ hostChatId: scopeChatId ?? chatId ?? null },
      })
        .unwrap()
        .then((result) => {
          if (Array.isArray(result)) result.forEach(deliverSource);
        })
        .catch(() => undefined);
    const acceptState = (result: unknown) => {
      if (
        !isAppRecord(result) ||
        !Number.isSafeInteger(result.revision) ||
        !("value" in result)
      )
        throw new Error("Invalid app state response");
      if (!active) throw new Error("App view changed");
      if (Number(result.revision) >= stateRevision) {
        const changed = Number(result.revision) > stateRevision;
        stateRevision = Number(result.revision);
        stateRecord.current = { revision: stateRevision, value: result.value };
        if (changed)
          bridge.notify("state", {
            state: result.value,
            revision: stateRevision,
          });
      }
      return { state: stateRecord.current?.value, revision: stateRevision };
    };
    const bridge = new AppBridgeHost({
      getFrame: () => iframe.current,
      channel,
      generation: 1,
      handlers: {
        ready: () => {
          setTimeout(() => {
            if (!active) return;
            appReady = true;
            beat(true);
            void refreshSources();
          }, 0);
          return Promise.resolve({
            app_id: appId,
            binding_id: bindingId,
            chat_id: chatId,
            appearance: theme.current,
            tokens: tokens.current ?? readTokens(),
            state: stateRecord.current?.value,
            revision: stateRevision,
          });
        },
        "state.get": async () =>
          acceptState(
            await command({
              action: "state_get",
              app_id: appId,
              scope,
              chat_id: chatId,
              ...credentials,
            }).unwrap(),
          ),
        "state.set": async (params: unknown) => {
          if (!isAppRecord(params) || !("state" in params))
            throw new Error("Invalid state");
          return acceptState(
            await command({
              action: "state_set",
              app_id: appId,
              scope,
              chat_id: chatId,
              ...credentials,
              expected_revision: stateRevision,
              value: params.state,
            }).unwrap(),
          );
        },
        "event.emit": async (params: unknown) => {
          if (!isAppRecord(params) || typeof params.name !== "string")
            throw new Error("Invalid event");
          return command({
            action: "event_emit",
            ...credentials,
            channel: params.name,
            data: params.payload,
          }).unwrap();
        },
        "action.request": async (params: unknown) => {
          if (!isAppRecord(params) || typeof params.name !== "string")
            throw new Error("Invalid action");
          return command({
            action: "action_request",
            ...credentials,
            request_id: uuidv4(),
            action_id: params.name,
            payload: params.payload,
          }).unwrap();
        },
        "context.stage": async (params: unknown) => {
          if (!chatId || !bindingId)
            throw new Error("Attach this app to a chat before staging context");
          if (
            !isAppRecord(params) ||
            typeof params.text !== "string" ||
            params.text.length > 8192
          )
            throw new Error("Invalid selected context");
          const result = await command({
            action: "context_stage",
            ...credentials,
            text: params.text,
            label:
              typeof params.label === "string"
                ? params.label.slice(0, 80)
                : undefined,
          }).unwrap();
          if (
            !isAppRecord(result) ||
            typeof result.context_id !== "string" ||
            typeof result.label !== "string" ||
            typeof result.text !== "string" ||
            typeof result.captured_at !== "string" ||
            result.app_id !== appId ||
            result.binding_id !== bindingId ||
            typeof result.revision !== "string"
          )
            throw new Error("Invalid staged context response");
          const context = {
            id: result.context_id,
            app_id: appId,
            binding_id: bindingId,
            revision: result.revision,
            view_id: identity.view_id,
            captured_at: result.captured_at,
            title: result.label,
            text: result.text,
          };
          if (!canStageAppContext(staged.current ?? [], context))
            throw new Error("Selected app context limit reached");
          dispatch(
            appContextStaged({
              chatId,
              context,
            }),
          );
          return Promise.resolve({ staged: true });
        },
      },
    });
    bridgeRef.current = bridge;
    bridge.attach(window);
    beat(true);
    const heartbeat = setInterval(() => beat(true), 20000);
    const presenceChanged = () => setTimeout(() => beat(false), 0);
    // Retained workspace tabs toggle ancestors' hidden attribute, not document
    // visibility. Observe only this surface's ancestor chain, never its draft DOM.
    const presenceObserver = new MutationObserver(() => beat(false));
    for (
      let node: HTMLElement | null = root.current;
      node;
      node = node.parentElement
    ) {
      presenceObserver.observe(node, {
        attributes: true,
        attributeFilter: ["hidden", "style", "class"],
      });
    }
    window.document.addEventListener("visibilitychange", presenceChanged);
    window.addEventListener("focus", presenceChanged);
    window.addEventListener("blur", presenceChanged);
    const unlisten = listenAppEvents((event) => {
      if ((event.hostChatId ?? undefined) !== (scopeChatId ?? chatId)) return;
      if (event.kind === "snapshot" || event.kind === "reset") {
        void refreshSources();
        return;
      }
      if (event.kind === "catalog_changed") {
        const previousRevision = documentRef.current?.revision;
        void refetch().then((result) => {
          if (result.data && result.data.revision !== previousRevision)
            setRegistrationEpoch((epoch) => epoch + 1);
        });
        return;
      }
      if (!isAppRecord(event.data)) return;
      if (
        event.kind === "view_request" &&
        event.data.view_id === identity.view_id &&
        event.data.generation === identity.generation
      ) {
        if (event.data.method === "focus") {
          controls.current.onFocus?.();
          iframe.current?.focus();
          presenceChanged();
        } else if (event.data.method === "close") controls.current.onClose?.();
      }
      if (event.kind === "source") deliverSource(event.data);
      if (
        event.kind === "state_changed" &&
        event.data.app_id === appId &&
        isAppRecord(event.data.scope) &&
        ((bindingId !== undefined &&
          event.data.scope.binding_id === bindingId) ||
          (!bindingId && event.data.scope.type === "standalone"))
      ) {
        void command({
          action: "state_get",
          app_id: appId,
          scope,
          chat_id: chatId,
          ...credentials,
        })
          .unwrap()
          .then(acceptState)
          .catch(() => {
            if (active) setNotice({ text: "App state refresh failed." });
          });
      }
      if (
        event.kind === "view_request" &&
        event.data.view_id === identity.view_id &&
        event.data.generation === identity.generation &&
        event.data.method === "capture" &&
        typeof event.data.request_id === "string"
      ) {
        const requestId = event.data.request_id;
        const reject = (message: string) => {
          if (active) setNotice({ text: `Screenshot failed: ${message}` });
          void command({
            action: "capture_response",
            ...credentials,
            request_id: requestId,
            data_url: null,
            error: message.slice(0, 200),
          })
            .unwrap()
            .catch(() => undefined);
        };
        if (!bridge.isReady) {
          reject("app is not ready");
          return;
        }
        void bridge
          .request("capture", {}, 15000)
          .then(async (result: unknown) => {
            if (!active) return;
            const capture = parseAppCaptureResult(result);
            if (!capture) throw new Error("invalid app capture");
            const { dataUrl, ...metadata } = capture;
            try {
              await command({
                action: "capture_response",
                ...credentials,
                request_id: requestId,
                data_url: dataUrl ?? null,
                metadata,
              }).unwrap();
            } catch {
              // Cleanup can change active while the upload is awaiting.
              // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
              if (active) setNotice({ text: "Screenshot upload failed." });
            }
          })
          .catch((cause: unknown) =>
            reject(
              cause instanceof Error && cause.message
                ? cause.message
                : "capture failed or is unsupported",
            ),
          );
      }
      if (
        event.kind === "app_event" &&
        event.data.app_id === appId &&
        event.data.binding_id === bindingId
      )
        bridge.notify("data", { name: "event", payload: event.data });
    });
    return () => {
      active = false;
      clearInterval(heartbeat);
      presenceObserver.disconnect();
      window.document.removeEventListener("visibilitychange", presenceChanged);
      window.removeEventListener("focus", presenceChanged);
      window.removeEventListener("blur", presenceChanged);
      unlisten();
      bridgeRef.current = null;
      bridge.detach();
    };
  }, [
    identity,
    documentReady,
    channel,
    appId,
    bindingId,
    chatId,
    scopeChatId,
    command,
    dispatch,
    refetch,
  ]);
  let content;
  if (error !== undefined || failure !== null)
    content = (
      <div role="alert" className={styles.feedback}>
        <Text>
          {failure ?? "This app is unavailable. Its binding is preserved."}
        </Text>
        <Button
          onClick={() => {
            setFailure(null);
            setRegistrationEpoch((epoch) => epoch + 1);
            void refetch();
          }}
        >
          Retry
        </Button>
      </div>
    );
  else if (pending || isLoading || !source)
    content = (
      <div role="status" className={styles.feedback}>
        Loading app…
      </div>
    );
  else
    content = (
      <div className={styles.surface}>
        {notice !== null && (
          <Flex role="status" align="center" gap="2" p="2" flexShrink="0">
            <Text size="1">{notice.text}</Text>
            {notice.reconnect && (
              <Button
                size="1"
                variant="soft"
                onClick={() => setRegistrationEpoch((epoch) => epoch + 1)}
              >
                Reconnect
              </Button>
            )}
            <Button size="1" variant="ghost" onClick={() => setNotice(null)}>
              Dismiss
            </Button>
          </Flex>
        )}
        <iframe
          ref={iframe}
          title={appId}
          className={styles.frame}
          sandbox={APP_IFRAME_SANDBOX}
          referrerPolicy="no-referrer"
          srcDoc={source}
        />
      </div>
    );
  return (
    <div ref={root} className={styles.root} data-app-surface={appId}>
      {content}
    </div>
  );
}
export function WorkspaceAppSurface({ surfaceKey }: { surfaceKey: string }) {
  const dispatch = useAppDispatch();
  const view = useAppSelector((state) => state.apps.views[surfaceKey]);
  return view ? (
    <AppSurface
      appId={view.appId}
      bindingId={view.bindingId}
      chatId={view.chatId}
      scopeChatId={view.scopeChatId}
      onFocus={() => dispatch(setActiveTab(surfaceKey))}
      onClose={() => dispatch(closeTab(surfaceKey))}
    />
  ) : (
    <div role="alert" className={styles.feedback}>
      App binding unavailable. Reopen its owning chat to restore it.
    </div>
  );
}
