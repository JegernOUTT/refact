export type DesignTheme = "light" | "dark";

export type DesignElementSelection = {
  selector: string;
  role: string;
  name: string;
  rect: {
    x: number;
    y: number;
    width: number;
    height: number;
  };
  computedStyles: Record<string, string>;
  sourceFile: string | null;
  line: number | null;
  cropDataUrl: string | null;
};

export type DesignSurfaceInboundMessage =
  | {
      type: "refact:design-ready";
      payload: { resourceUri: string };
    }
  | {
      type: "refact:element-selected";
      payload: DesignElementSelection;
    }
  | {
      type: "refact:iframe-blocked";
      payload: { reason: string };
    };

export type DesignSurfaceOutboundMessage =
  | {
      type: "refact:set-state";
      payload: {
        theme: DesignTheme;
        pickerEnabled: boolean;
        devicePixelRatio: number;
      };
    }
  | {
      type: "refact:call-tool";
      payload: { name: string; arguments: Record<string, unknown> };
    }
  | {
      type: "refact:send-followup-turn";
      payload: { content: string };
    };

export interface SurfaceRenderer<TState> {
  readonly ui: { readonly resourceUri: string };
  callTool(name: string, args: Record<string, unknown>): void;
  sendFollowupTurn(content: string): void;
  setState(state: TState): void;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const isFiniteNumber = (value: unknown): value is number =>
  typeof value === "number" && Number.isFinite(value);

const isNullableString = (value: unknown): value is string | null =>
  typeof value === "string" || value === null;

function isElementSelection(value: unknown): value is DesignElementSelection {
  if (!isRecord(value) || !isRecord(value.rect)) return false;
  return (
    typeof value.selector === "string" &&
    typeof value.role === "string" &&
    typeof value.name === "string" &&
    isFiniteNumber(value.rect.x) &&
    isFiniteNumber(value.rect.y) &&
    isFiniteNumber(value.rect.width) &&
    isFiniteNumber(value.rect.height) &&
    isRecord(value.computedStyles) &&
    Object.values(value.computedStyles).every(
      (style) => typeof style === "string",
    ) &&
    isNullableString(value.sourceFile) &&
    (isFiniteNumber(value.line) || value.line === null) &&
    isNullableString(value.cropDataUrl)
  );
}

export function parseDesignSurfaceMessage(
  value: unknown,
): DesignSurfaceInboundMessage | null {
  if (!isRecord(value) || typeof value.type !== "string") return null;
  if (value.type === "refact:design-ready") {
    return isRecord(value.payload) &&
      typeof value.payload.resourceUri === "string"
      ? {
          type: value.type,
          payload: { resourceUri: value.payload.resourceUri },
        }
      : null;
  }
  if (value.type === "refact:element-selected") {
    return isElementSelection(value.payload)
      ? { type: value.type, payload: value.payload }
      : null;
  }
  if (value.type === "refact:iframe-blocked") {
    return isRecord(value.payload) && typeof value.payload.reason === "string"
      ? { type: value.type, payload: { reason: value.payload.reason } }
      : null;
  }
  return null;
}
