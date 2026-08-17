import classNames from "classnames";
import { useEffect, useMemo, useRef } from "react";

import { wrapArtifactHtml } from "../../components/Markdown/renderUtils";
import type { BrowserFrame } from "../Browser/browserSlice";
import { createDesignChannel } from "./channel";
import type { DesignSurfaceState } from "./designSlice";
import {
  HOST_ORIGIN_FALLBACK_REASON,
  resolveDesignFallbackReason,
  resolveDesignRenderer,
} from "./designRenderer";
import type { DesignSurfaceInboundMessage } from "./surfaceContract";
import styles from "./Design.module.css";

type DesignFrameProps = {
  browserFrame: BrowserFrame | null;
  state: DesignSurfaceState;
  onMessage: (message: DesignSurfaceInboundMessage) => void;
  onBasicLoad: () => void;
  onBlocked: (reason: string) => void;
};

const viewportWidth = (state: DesignSurfaceState): number =>
  state.viewportPreset === "custom"
    ? state.customWidth
    : Number(state.viewportPreset);

function LiveFrame({
  onBasicLoad,
  onBlocked,
  onMessage,
  state,
}: DesignFrameProps) {
  const frameRef = useRef<HTMLIFrameElement>(null);
  const loadTimerRef = useRef<number | null>(null);
  const origin = useMemo(() => {
    try {
      return new URL(state.liveUrl).origin;
    } catch {
      return null;
    }
  }, [state.liveUrl]);
  const sharesHostOrigin = origin === window.location.origin;

  useEffect(() => {
    if (sharesHostOrigin) onBlocked(HOST_ORIGIN_FALLBACK_REASON);
  }, [onBlocked, sharesHostOrigin]);

  useEffect(() => {
    if (sharesHostOrigin) return;
    loadTimerRef.current = window.setTimeout(() => {
      onBlocked(
        "The preview did not load. The server may deny framing with X-Frame-Options or Content-Security-Policy frame-ancestors.",
      );
    }, 5000);
    return () => {
      if (loadTimerRef.current !== null) {
        window.clearTimeout(loadTimerRef.current);
      }
    };
  }, [onBlocked, sharesHostOrigin, state.liveUrl, state.refreshNonce]);

  useEffect(() => {
    const frame = frameRef.current;
    if (!frame || !origin || sharesHostOrigin) return;
    const channel = createDesignChannel({
      frame,
      allowedOrigins: [origin],
      resourceUri: state.liveUrl,
      onMessage: (message) => {
        if (message.type === "refact:design-ready") {
          channel.setState({
            theme: state.theme,
            pickerEnabled: state.pickerEnabled,
            devicePixelRatio: state.devicePixelRatio,
          });
        }
        onMessage(message);
      },
    });
    channel.setState({
      theme: state.theme,
      pickerEnabled: state.pickerEnabled,
      devicePixelRatio: state.devicePixelRatio,
    });
    return () => channel.dispose();
  }, [
    onMessage,
    origin,
    sharesHostOrigin,
    state.devicePixelRatio,
    state.liveUrl,
    state.pickerEnabled,
    state.refreshNonce,
    state.theme,
  ]);

  const handleLoad = (): void => {
    if (loadTimerRef.current !== null) {
      window.clearTimeout(loadTimerRef.current);
      loadTimerRef.current = null;
    }
    onBasicLoad();
  };

  const handleError = (): void => {
    if (loadTimerRef.current !== null) {
      window.clearTimeout(loadTimerRef.current);
      loadTimerRef.current = null;
    }
    onBlocked(
      "The preview was refused by the host. Check X-Frame-Options and Content-Security-Policy frame-ancestors.",
    );
  };

  if (sharesHostOrigin) {
    return (
      <div className={styles.emptySource} role="status">
        {HOST_ORIGIN_FALLBACK_REASON}
      </div>
    );
  }

  return (
    <iframe
      key={`${state.liveUrl}:${state.refreshNonce}`}
      ref={frameRef}
      className={styles.frame}
      width={viewportWidth(state)}
      onError={handleError}
      onLoad={handleLoad}
      referrerPolicy="no-referrer"
      src={state.liveUrl}
      title="Live design preview"
    />
  );
}

function ArtifactFrame({ state }: { state: DesignSurfaceState }) {
  return (
    <iframe
      className={styles.frame}
      width={viewportWidth(state)}
      referrerPolicy="no-referrer"
      sandbox="allow-scripts"
      srcDoc={wrapArtifactHtml(state.artifactHtml)}
      title="Design artifact preview"
    />
  );
}

function ReferenceFrame({ state }: { state: DesignSurfaceState }) {
  if (!state.referenceDataUrl) {
    return <div className={styles.emptySource}>Choose a reference image.</div>;
  }
  let comparisonUrl: string | null = null;
  try {
    const url = new URL(state.liveUrl);
    if (
      (url.protocol === "http:" || url.protocol === "https:") &&
      url.origin !== window.location.origin
    ) {
      comparisonUrl = url.href;
    }
  } catch {
    comparisonUrl = null;
  }
  return (
    <div
      className={classNames(
        state.compareMode === "overlay"
          ? styles.referenceOverlay
          : styles.referenceSingle,
        styles[`opacity${state.overlayOpacity}`],
      )}
    >
      {comparisonUrl ? (
        <iframe
          className={styles.referenceLive}
          width={viewportWidth(state)}
          referrerPolicy="no-referrer"
          src={comparisonUrl}
          title="Live comparison preview"
        />
      ) : null}
      <img
        className={styles.referenceImage}
        width={viewportWidth(state)}
        src={state.referenceDataUrl}
        alt="Design reference"
      />
    </div>
  );
}

export function DesignFrame(props: DesignFrameProps) {
  const { browserFrame, state } = props;
  const renderer = resolveDesignRenderer(state);
  const zoomClass = styles[`zoom${state.zoom}`];

  if (renderer === "empty") {
    return (
      <div className={styles.emptySource}>Enter a development server URL.</div>
    );
  }
  if (renderer === "browser-frame") {
    return (
      <div className={styles.fallback} role="status">
        <p>{resolveDesignFallbackReason(state)}</p>
        <p>
          The host refused iframe embedding, so Design is showing the CDP
          screencast instead.
        </p>
        {browserFrame ? (
          <img
            src={`data:${browserFrame.mime};base64,${browserFrame.data}`}
            alt="CDP browser fallback frame"
          />
        ) : (
          <p>Start the chat browser to capture a fallback frame.</p>
        )}
      </div>
    );
  }

  return (
    <div className={styles.canvas} data-theme={state.theme}>
      <div className={classNames(styles.viewport, zoomClass)}>
        {renderer === "live-basic" || renderer === "live-interactive" ? (
          <LiveFrame {...props} />
        ) : null}
        {renderer === "artifact" ? <ArtifactFrame state={state} /> : null}
        {renderer === "reference" ? <ReferenceFrame state={state} /> : null}
      </div>
    </div>
  );
}
