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
    }
  | {
      type: "refact:send-followup-turn";
      payload: { content: string };
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
    };

export type PendingStyleEdit = {
  selector: string;
  styles: Record<string, string>;
};

export type DesignAnnotation = {
  id: string;
  selector: string;
  label: string;
};

export type DesignRuntimeOptions = {
  allowedParentOrigins: readonly string[];
  window?: Window;
  document?: Document;
  parentWindow?: Window;
};

export type DesignRuntime = {
  readonly handshaken: boolean;
  readonly pendingEdits: readonly PendingStyleEdit[];
  readonly annotations: readonly DesignAnnotation[];
  applyStyleEdit(edit: PendingStyleEdit): boolean;
  clearPendingStyleEdits(): void;
  addAnnotation(annotation: DesignAnnotation): boolean;
  removeAnnotation(id: string): void;
  apply(instruction: string, screenshot: string | null): boolean;
  dispose(): void;
};

type RuntimeState = Extract<
  DesignSurfaceOutboundMessage,
  { type: "refact:set-state" }
>["payload"];

type OriginalStyle = Map<string, string>;
type ToolCall = Extract<
  DesignSurfaceOutboundMessage,
  { type: "refact:call-tool" }
>["payload"];

const STYLE_PROPERTIES = [
  "display",
  "position",
  "color",
  "background-color",
  "font-family",
  "font-size",
  "font-weight",
  "line-height",
  "letter-spacing",
  "text-align",
  "width",
  "height",
  "min-width",
  "min-height",
  "max-width",
  "max-height",
  "margin",
  "padding",
  "gap",
  "border",
  "border-radius",
  "box-shadow",
  "opacity",
  "align-items",
  "justify-content",
  "grid-template-columns",
  "flex-direction",
] as const;

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

function parseStateMessage(value: unknown): RuntimeState | null {
  if (!isRecord(value) || value.type !== "refact:set-state" || !isRecord(value.payload)) {
    return null;
  }
  const { devicePixelRatio, pickerEnabled, theme } = value.payload;
  if (
    (theme !== "light" && theme !== "dark") ||
    typeof pickerEnabled !== "boolean" ||
    typeof devicePixelRatio !== "number" ||
    !Number.isFinite(devicePixelRatio)
  ) {
    return null;
  }
  return { theme, pickerEnabled, devicePixelRatio };
}

function parseToolCall(value: unknown): ToolCall | null {
  if (
    !isRecord(value) ||
    value.type !== "refact:call-tool" ||
    !isRecord(value.payload) ||
    typeof value.payload.name !== "string" ||
    !isRecord(value.payload.arguments)
  ) {
    return null;
  }
  return {
    name: value.payload.name,
    arguments: value.payload.arguments,
  };
}

function stringRecord(value: unknown): Record<string, string> | null {
  if (!isRecord(value)) return null;
  return Object.values(value).every((item) => typeof item === "string")
    ? (value as Record<string, string>)
    : null;
}

function parseSource(element: Element): { sourceFile: string | null; line: number | null } {
  const value = element.getAttribute("data-refact-src");
  if (!value) return { sourceFile: null, line: null };
  const match = /^(.*):(\d+):(\d+)$/.exec(value);
  if (!match) return { sourceFile: value, line: null };
  return {
    sourceFile: match[1] ?? null,
    line: Number(match[2]),
  };
}

function escapeIdentifier(value: string): string {
  const escape = globalThis.CSS?.escape;
  if (escape) return escape(value);
  return value.replaceAll(/[^a-zA-Z0-9_-]/g, (character) => `\\${character}`);
}

function escapeAttributeValue(value: string): string {
  return value.replaceAll("\\", "\\\\").replaceAll('"', '\\"');
}

function selectorFor(element: Element): string {
  const source = element.getAttribute("data-refact-src");
  if (source) return `[data-refact-src="${escapeAttributeValue(source)}"]`;
  if (element.id) return `#${escapeIdentifier(element.id)}`;
  const parts: string[] = [];
  let current: Element | null = element;
  while (current && current !== current.ownerDocument.documentElement) {
    const tag = current.tagName.toLowerCase();
    const siblings = current.parentElement
      ? [...current.parentElement.children].filter((child) => child.tagName === current?.tagName)
      : [];
    const suffix =
      siblings.length > 1 ? `:nth-of-type(${siblings.indexOf(current) + 1})` : "";
    parts.unshift(`${tag}${suffix}`);
    current = current.parentElement;
  }
  return parts.join(" > ");
}

function implicitRole(element: Element): string {
  const explicitRole = element.getAttribute("role");
  if (explicitRole) return explicitRole;
  const tag = element.tagName.toLowerCase();
  if (tag === "button") return "button";
  if (tag === "a" && element.hasAttribute("href")) return "link";
  if (tag === "img") return "img";
  if (tag === "textarea") return "textbox";
  if (tag === "input") {
    const type = element.getAttribute("type") ?? "text";
    if (type === "checkbox") return "checkbox";
    if (type === "radio") return "radio";
    if (type === "button" || type === "submit" || type === "reset") return "button";
    return "textbox";
  }
  return "";
}

function accessibleName(element: Element): string {
  return (
    element.getAttribute("aria-label") ??
    element.getAttribute("alt") ??
    element.getAttribute("title") ??
    element.textContent?.trim().replaceAll(/\s+/g, " ").slice(0, 160) ??
    ""
  );
}

function computedStyleRecord(view: Window, element: Element): Record<string, string> {
  const computed = view.getComputedStyle(element);
  return Object.fromEntries(
    STYLE_PROPERTIES.map((property) => [property, computed.getPropertyValue(property)]),
  );
}

function selectionFor(view: Window, element: Element): DesignElementSelection {
  const rect = element.getBoundingClientRect();
  const source = parseSource(element);
  return {
    selector: selectorFor(element),
    role: implicitRole(element),
    name: accessibleName(element),
    rect: {
      x: rect.x,
      y: rect.y,
      width: rect.width,
      height: rect.height,
    },
    computedStyles: computedStyleRecord(view, element),
    sourceFile: source.sourceFile,
    line: source.line,
    cropDataUrl: null,
  };
}

function validStyle(property: string, value: string): boolean {
  return property.startsWith("--") || globalThis.CSS?.supports?.(property, value) === true;
}

function makeOverlay(documentValue: Document): {
  root: HTMLDivElement;
  highlight: HTMLDivElement;
  pins: HTMLDivElement;
} {
  const root = documentValue.createElement("div");
  root.dataset.refactDesignOverlay = "true";
  root.style.cssText =
    "position:fixed;inset:0;pointer-events:none;z-index:2147483647;font:12px sans-serif";
  const highlight = documentValue.createElement("div");
  highlight.style.cssText =
    "position:fixed;display:none;border:2px solid #7c3aed;background:rgba(124,58,237,.12);box-sizing:border-box";
  const pins = documentValue.createElement("div");
  root.append(highlight, pins);
  documentValue.documentElement.append(root);
  return { root, highlight, pins };
}

function positionHighlight(highlight: HTMLDivElement, element: Element | null): void {
  if (!element) {
    highlight.style.display = "none";
    return;
  }
  const rect = element.getBoundingClientRect();
  highlight.style.display = "block";
  highlight.style.left = `${rect.left}px`;
  highlight.style.top = `${rect.top}px`;
  highlight.style.width = `${rect.width}px`;
  highlight.style.height = `${rect.height}px`;
}

export function createDesignRuntime(options: DesignRuntimeOptions): DesignRuntime {
  const view = options.window ?? window;
  const documentValue = options.document ?? view.document;
  const parentWindow = options.parentWindow ?? view.parent;
  const embedded = parentWindow !== view;
  const origins = new Set(
    options.allowedParentOrigins.map((origin) => new URL(origin).origin),
  );
  if (origins.size === 0) {
    throw new Error("Refact Design runtime requires at least one allowed parent origin");
  }
  const pendingEdits = new Map<string, Map<string, string>>();
  const originalStyles = new Map<HTMLElement, OriginalStyle>();
  const annotations = new Map<string, DesignAnnotation>();
  let state: RuntimeState | null = null;
  let parentOrigin: string | null = null;
  let hovered: Element | null = null;
  let overlay: ReturnType<typeof makeOverlay> | null = null;

  const post = (message: DesignSurfaceInboundMessage): boolean => {
    if (!parentOrigin) return false;
    parentWindow.postMessage(message, parentOrigin);
    return true;
  };

  const renderAnnotations = (): void => {
    if (!overlay) return;
    overlay.pins.replaceChildren();
    for (const annotation of annotations.values()) {
      const target = documentValue.querySelector(annotation.selector);
      if (!target) continue;
      const rect = target.getBoundingClientRect();
      const pin = documentValue.createElement("div");
      pin.dataset.refactAnnotationId = annotation.id;
      pin.textContent = annotation.label;
      pin.style.cssText =
        "position:fixed;padding:3px 7px;border-radius:999px;color:white;background:#7c3aed;box-shadow:0 2px 8px rgba(0,0,0,.25);max-width:220px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap";
      pin.style.left = `${rect.right - 8}px`;
      pin.style.top = `${rect.top - 8}px`;
      overlay.pins.append(pin);
    }
  };

  const handlePointerMove = (event: PointerEvent): void => {
    if (!state?.pickerEnabled || !overlay) return;
    const target = event.target instanceof Element ? event.target : null;
    if (!target || target.closest("[data-refact-design-overlay]")) return;
    hovered = target;
    positionHighlight(overlay.highlight, target);
  };

  const handleClick = (event: MouseEvent): void => {
    if (!state?.pickerEnabled || !hovered) return;
    event.preventDefault();
    event.stopPropagation();
    post({
      type: "refact:element-selected",
      payload: selectionFor(view, hovered),
    });
  };

  const handleLayout = (): void => {
    if (overlay) positionHighlight(overlay.highlight, state?.pickerEnabled ? hovered : null);
    renderAnnotations();
  };

  const activate = (): void => {
    if (overlay) return;
    overlay = makeOverlay(documentValue);
    documentValue.addEventListener("pointermove", handlePointerMove, true);
    documentValue.addEventListener("click", handleClick, true);
    view.addEventListener("scroll", handleLayout, true);
    view.addEventListener("resize", handleLayout);
  };

  const deactivate = (): void => {
    documentValue.removeEventListener("pointermove", handlePointerMove, true);
    documentValue.removeEventListener("click", handleClick, true);
    view.removeEventListener("scroll", handleLayout, true);
    view.removeEventListener("resize", handleLayout);
    overlay?.root.remove();
    overlay = null;
    hovered = null;
  };

  const handleMessage = (event: MessageEvent): void => {
    if (!embedded || event.source !== parentWindow || !origins.has(event.origin)) return;
    const nextState = parseStateMessage(event.data);
    if (nextState) {
      const firstHandshake = state === null;
      state = nextState;
      parentOrigin = event.origin;
      activate();
      documentValue.documentElement.dataset.refactDesignTheme = nextState.theme;
      if (!nextState.pickerEnabled && overlay) {
        hovered = null;
        positionHighlight(overlay.highlight, null);
      }
      if (firstHandshake) {
        post({
          type: "refact:design-ready",
          payload: { resourceUri: view.location.href },
        });
      }
      return;
    }
    if (!state) return;
    const toolCall = parseToolCall(event.data);
    if (!toolCall) return;
    const args = toolCall.arguments;
    if (toolCall.name === "design.apply-style-edit") {
      const styles = stringRecord(args.styles);
      if (typeof args.selector === "string" && styles) {
        runtime.applyStyleEdit({ selector: args.selector, styles });
      }
    } else if (toolCall.name === "design.clear-style-edits") {
      runtime.clearPendingStyleEdits();
    } else if (toolCall.name === "design.add-annotation") {
      if (
        typeof args.id === "string" &&
        typeof args.selector === "string" &&
        typeof args.label === "string"
      ) {
        runtime.addAnnotation({
          id: args.id,
          selector: args.selector,
          label: args.label,
        });
      }
    } else if (toolCall.name === "design.remove-annotation") {
      if (typeof args.id === "string") runtime.removeAnnotation(args.id);
    } else if (toolCall.name === "design.apply") {
      if (
        typeof args.instruction === "string" &&
        (typeof args.screenshot === "string" || args.screenshot === null)
      ) {
        runtime.apply(args.instruction, args.screenshot);
      }
    }
  };

  const runtime: DesignRuntime = {
    get handshaken() {
      return state !== null;
    },
    get pendingEdits() {
      return [...pendingEdits].map(([selector, styles]) => ({
        selector,
        styles: Object.fromEntries(styles),
      }));
    },
    get annotations() {
      return [...annotations.values()];
    },
    applyStyleEdit(edit) {
      if (!state) return false;
      const element = documentValue.querySelector<HTMLElement>(edit.selector);
      if (!element) return false;
      const original = originalStyles.get(element) ?? new Map<string, string>();
      const pending = pendingEdits.get(edit.selector) ?? new Map<string, string>();
      let applied = false;
      for (const [property, value] of Object.entries(edit.styles)) {
        if (!validStyle(property, value)) continue;
        if (!original.has(property)) original.set(property, element.style.getPropertyValue(property));
        element.style.setProperty(property, value);
        pending.set(property, value);
        applied = true;
      }
      if (applied) {
        originalStyles.set(element, original);
        pendingEdits.set(edit.selector, pending);
        handleLayout();
      }
      return applied;
    },
    clearPendingStyleEdits() {
      for (const [element, styles] of originalStyles) {
        for (const [property, value] of styles) {
          if (value) element.style.setProperty(property, value);
          else element.style.removeProperty(property);
        }
      }
      originalStyles.clear();
      pendingEdits.clear();
      handleLayout();
    },
    addAnnotation(annotation) {
      if (!state || !documentValue.querySelector(annotation.selector)) return false;
      annotations.set(annotation.id, { ...annotation });
      renderAnnotations();
      return true;
    },
    removeAnnotation(id) {
      annotations.delete(id);
      renderAnnotations();
    },
    apply(instruction, screenshot) {
      if (!state || pendingEdits.size === 0) return false;
      const content = JSON.stringify({
        edits: runtime.pendingEdits,
        instruction,
        screenshot,
      });
      return post({
        type: "refact:send-followup-turn",
        payload: { content },
      });
    },
    dispose() {
      view.removeEventListener("message", handleMessage);
      runtime.clearPendingStyleEdits();
      annotations.clear();
      deactivate();
      state = null;
      parentOrigin = null;
    },
  };
  if (embedded) view.addEventListener("message", handleMessage);
  return runtime;
}
