import {
  parseDesignSurfaceMessage,
  type DesignSurfaceInboundMessage,
  type DesignSurfaceOutboundMessage,
  type SurfaceRenderer,
} from "./surfaceContract";

export type DesignChannelOptions = {
  frame: HTMLIFrameElement;
  allowedOrigins: readonly string[];
  resourceUri: string;
  onMessage: (message: DesignSurfaceInboundMessage) => void;
};

export type DesignChannel = SurfaceRenderer<
  Extract<DesignSurfaceOutboundMessage, { type: "refact:set-state" }>["payload"]
> & {
  dispose(): void;
};

export function isAllowedDesignMessage(
  event: Pick<MessageEvent, "origin" | "source">,
  frameWindow: Window | null,
  allowedOrigins: ReadonlySet<string>,
): boolean {
  return (
    frameWindow !== null &&
    event.source === frameWindow &&
    allowedOrigins.has(event.origin)
  );
}

export function createDesignChannel({
  allowedOrigins,
  frame,
  onMessage,
  resourceUri,
}: DesignChannelOptions): DesignChannel {
  const origins = new Set(allowedOrigins);
  const targetOrigin = allowedOrigins[0];

  const post = (message: DesignSurfaceOutboundMessage): void => {
    if (!targetOrigin || !origins.has(targetOrigin)) return;
    frame.contentWindow?.postMessage(message, targetOrigin);
  };
  const handleMessage = (event: MessageEvent): void => {
    if (!isAllowedDesignMessage(event, frame.contentWindow, origins)) return;
    const message = parseDesignSurfaceMessage(event.data);
    if (message) onMessage(message);
  };

  window.addEventListener("message", handleMessage);
  return {
    ui: { resourceUri },
    callTool: (name, args) =>
      post({ type: "refact:call-tool", payload: { name, arguments: args } }),
    setState: (state) => post({ type: "refact:set-state", payload: state }),
    dispose: () => window.removeEventListener("message", handleMessage),
  };
}
