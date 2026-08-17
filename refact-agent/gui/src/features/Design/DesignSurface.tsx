import { useCallback, useEffect } from "react";

import {
  Field,
  FieldText,
  FieldTextarea,
  StatusDot,
} from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { useChatActions } from "../../hooks/useChatActions";
import { selectBrowserRuntime } from "../Browser/browserSlice";
import { selectSurfaceChatId } from "../Workspace/workspaceSlice";
import { DesignFrame } from "./DesignFrame";
import { DesignToolbar } from "./DesignToolbar";
import {
  ensureDesignSurface,
  makeDesignSurfaceState,
  refreshDesignSurface,
  selectDesignSurface,
  updateDesignSurface,
  type DesignSurfaceState,
} from "./designSlice";
import { PickerOverlay } from "./PickerOverlay";
import type { DesignSurfaceInboundMessage } from "./surfaceContract";
import styles from "./Design.module.css";

type DesignSurfaceProps = {
  surfaceId: string;
};

export function DesignSurface({ surfaceId }: DesignSurfaceProps) {
  const dispatch = useAppDispatch();
  const storedState = useAppSelector((state) =>
    selectDesignSurface(state, surfaceId),
  );
  const chatId = useAppSelector((state) =>
    selectSurfaceChatId(state, `design:${surfaceId}`),
  );
  const browserRuntime = useAppSelector((state) =>
    chatId ? selectBrowserRuntime(state, chatId) : undefined,
  );
  const { submit } = useChatActions(chatId ?? undefined);
  const state = storedState ?? makeDesignSurfaceState();

  useEffect(() => {
    dispatch(ensureDesignSurface(surfaceId));
  }, [dispatch, surfaceId]);

  const onPatch = useCallback(
    (patch: Partial<DesignSurfaceState>) => {
      dispatch(updateDesignSurface({ surfaceId, patch }));
    },
    [dispatch, surfaceId],
  );
  const handleMessage = useCallback(
    (message: DesignSurfaceInboundMessage) => {
      if (message.type === "refact:design-ready") {
        onPatch({ liveStatus: "interactive", fallbackReason: null });
      } else if (message.type === "refact:element-selected") {
        onPatch({ selection: message.payload });
      } else if (message.type === "refact:iframe-blocked") {
        onPatch({
          liveStatus: "blocked",
          fallbackReason: message.payload.reason,
        });
      } else {
        void submit(message.payload.content);
      }
    },
    [onPatch, submit],
  );
  const handleReference = useCallback(
    (files: FileList | null) => {
      const file = files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.addEventListener("load", () => {
        if (typeof reader.result === "string") {
          onPatch({ referenceDataUrl: reader.result });
        }
      });
      reader.readAsDataURL(file);
    },
    [onPatch],
  );
  const handleBasicLoad = useCallback(() => {
    onPatch({ liveStatus: "basic" });
  }, [onPatch]);
  const handleBlocked = useCallback(
    (fallbackReason: string) => {
      onPatch({ liveStatus: "blocked", fallbackReason });
    },
    [onPatch],
  );

  return (
    <section className={styles.surface} aria-label="Design surface">
      <DesignToolbar
        state={state}
        onPatch={onPatch}
        onRefresh={() => dispatch(refreshDesignSurface(surfaceId))}
      />
      <div className={styles.sourceConfiguration}>
        {state.source === "live" ? (
          <Field label="Development server URL">
            <FieldText
              aria-label="Development server URL"
              placeholder="http://localhost:5173"
              value={state.liveUrl}
              onChange={(liveUrl) =>
                onPatch({
                  liveUrl,
                  liveStatus: liveUrl ? "probing" : "idle",
                  fallbackReason: null,
                })
              }
            />
          </Field>
        ) : null}
        {state.source === "artifact" ? (
          <Field label="Artifact HTML">
            <FieldTextarea
              aria-label="Artifact HTML"
              rows={3}
              value={state.artifactHtml}
              onChange={(artifactHtml) => onPatch({ artifactHtml })}
            />
          </Field>
        ) : null}
        {state.source === "reference" ? (
          <Field label="Reference image">
            <input
              aria-label="Reference image"
              accept="image/*"
              className={styles.fileInput}
              type="file"
              onChange={(event) => handleReference(event.currentTarget.files)}
            />
          </Field>
        ) : null}
      </div>
      {state.source === "live" && state.liveStatus === "basic" ? (
        <div className={styles.notice} role="status">
          <StatusDot status="idle" size="small" />
          The app renders, but @refact/vite-plugin-design was not detected. The
          picker and source mapping are unavailable; use the CDP browser for
          probing.
        </div>
      ) : null}
      <DesignFrame
        browserFrame={browserRuntime?.latest_frame ?? null}
        state={state}
        onBasicLoad={handleBasicLoad}
        onBlocked={handleBlocked}
        onMessage={handleMessage}
      />
      <PickerOverlay selection={state.selection} />
    </section>
  );
}
