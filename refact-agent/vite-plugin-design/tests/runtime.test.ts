import { beforeEach, describe, expect, it, vi } from "vitest";

import { createDesignRuntime } from "../src/runtime";

const allowedOrigin = "http://127.0.0.1:8001";
const stateMessage = {
  type: "refact:set-state",
  payload: {
    theme: "dark",
    pickerEnabled: true,
    devicePixelRatio: 2,
  },
};

let parentWindow: Window;
let postMessage: ReturnType<typeof vi.fn>;

function handshake(origin = allowedOrigin): void {
  window.dispatchEvent(
    new MessageEvent("message", {
      data: stateMessage,
      origin,
      source: parentWindow,
    }),
  );
}

function callTool(name: string, args: Record<string, unknown>): void {
  window.dispatchEvent(
    new MessageEvent("message", {
      data: {
        type: "refact:call-tool",
        payload: { name, arguments: args },
      },
      origin: allowedOrigin,
      source: parentWindow,
    }),
  );
}

describe("createDesignRuntime", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    document.body.replaceChildren();
    document.documentElement.removeAttribute("data-refact-design-theme");
    postMessage = vi.fn();
    parentWindow = { postMessage } as unknown as Window;
  });

  it("rejects an empty parent-origin allowlist", () => {
    expect(() => createDesignRuntime({ allowedParentOrigins: [] })).toThrow(
      "requires at least one allowed parent origin",
    );
  });

  it("stays inert until an allowed parent handshakes", () => {
    const runtime = createDesignRuntime({
      allowedParentOrigins: [allowedOrigin],
      parentWindow,
    });

    expect(runtime.handshaken).toBe(false);
    expect(document.querySelector("[data-refact-design-overlay]")).toBeNull();
    expect(runtime.applyStyleEdit({ selector: "body", styles: { color: "red" } })).toBe(
      false,
    );

    runtime.dispose();
  });

  it("installs no listener when opened as a top-level app", () => {
    const addEventListener = vi.spyOn(window, "addEventListener");
    const runtime = createDesignRuntime({ allowedParentOrigins: [allowedOrigin] });

    expect(runtime.handshaken).toBe(false);
    expect(addEventListener).not.toHaveBeenCalled();
    runtime.dispose();
  });

  it("rejects a disallowed parent origin", () => {
    const runtime = createDesignRuntime({
      allowedParentOrigins: [allowedOrigin],
      parentWindow,
    });

    handshake("https://blocked.example");

    expect(runtime.handshaken).toBe(false);
    expect(document.querySelector("[data-refact-design-overlay]")).toBeNull();
    expect(postMessage).not.toHaveBeenCalled();
    runtime.dispose();
  });

  it("activates picker behavior and sends the exact element payload", () => {
    const button = document.createElement("button");
    button.dataset.refactSrc = "src/App.tsx:42:7";
    button.dataset.refactCmp = "App";
    button.textContent = "Save";
    document.body.append(button);
    const runtime = createDesignRuntime({
      allowedParentOrigins: [allowedOrigin],
      parentWindow,
    });

    handshake();
    button.dispatchEvent(new PointerEvent("pointermove", { bubbles: true }));
    button.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));

    expect(runtime.handshaken).toBe(true);
    expect(document.documentElement.dataset.refactDesignTheme).toBe("dark");
    expect(postMessage).toHaveBeenCalledWith(
      {
        type: "refact:element-selected",
        payload: expect.objectContaining({
          selector: '[data-refact-src="src/App.tsx:42:7"]',
          role: "button",
          name: "Save",
          rect: expect.objectContaining({
            x: expect.any(Number),
            y: expect.any(Number),
            width: expect.any(Number),
            height: expect.any(Number),
          }),
          computedStyles: expect.any(Object),
          sourceFile: "src/App.tsx",
          line: 42,
          cropDataUrl: null,
        }),
      },
      allowedOrigin,
    );
    runtime.dispose();
  });

  it("keeps style edits inline, renders pins, and serializes Apply once", () => {
    const target = document.createElement("div");
    target.id = "target";
    document.body.append(target);
    const runtime = createDesignRuntime({
      allowedParentOrigins: [allowedOrigin],
      parentWindow,
    });
    handshake();

    callTool("design.apply-style-edit", {
      selector: "#target",
      styles: { color: "rgb(255, 0, 0)", padding: "8px" },
    });
    expect(target.style.color).toBe("rgb(255, 0, 0)");
    expect(runtime.pendingEdits).toEqual([
      {
        selector: "#target",
        styles: { color: "rgb(255, 0, 0)", padding: "8px" },
      },
    ]);
    callTool("design.add-annotation", {
      id: "pin-1",
      selector: "#target",
      label: "Tighten spacing",
    });
    expect(
      document.querySelector('[data-refact-annotation-id="pin-1"]')?.textContent,
    ).toBe("Tighten spacing");

    callTool("design.apply", {
      instruction: "Match the reference",
      screenshot: "data:image/png;base64,crop",
    });
    const calls = postMessage.mock.calls.filter(
      ([message]) =>
        typeof message === "object" &&
        message !== null &&
        "type" in message &&
        message.type === "refact:send-followup-turn",
    );
    expect(calls).toHaveLength(1);
    const message = calls[0]?.[0] as { payload: { content: string } };
    expect(JSON.parse(message.payload.content)).toEqual({
      edits: runtime.pendingEdits,
      instruction: "Match the reference",
      screenshot: "data:image/png;base64,crop",
    });

    runtime.clearPendingStyleEdits();
    expect(target.style.color).toBe("");
    expect(runtime.pendingEdits).toEqual([]);
    runtime.dispose();
  });
});
