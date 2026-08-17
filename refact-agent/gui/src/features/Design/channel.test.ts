import { afterEach, describe, expect, it, vi } from "vitest";

import { createDesignChannel, isAllowedDesignMessage } from "./channel";

afterEach(() => {
  vi.restoreAllMocks();
});

describe("Design channel", () => {
  it("requires both an allowed origin and the expected frame source", () => {
    const frameWindow = window;
    const allowedOrigins = new Set(["https://allowed.example"]);

    expect(
      isAllowedDesignMessage(
        { origin: "https://allowed.example", source: frameWindow },
        frameWindow,
        allowedOrigins,
      ),
    ).toBe(true);
    expect(
      isAllowedDesignMessage(
        { origin: "https://blocked.example", source: frameWindow },
        frameWindow,
        allowedOrigins,
      ),
    ).toBe(false);
    expect(
      isAllowedDesignMessage(
        { origin: "https://allowed.example", source: null },
        frameWindow,
        allowedOrigins,
      ),
    ).toBe(false);
  });

  it("rejects inbound messages from a disallowed origin", () => {
    const onMessage = vi.fn();
    const frame = document.createElement("iframe");
    document.body.append(frame);
    const frameWindow = frame.contentWindow;
    expect(frameWindow).not.toBeNull();
    const channel = createDesignChannel({
      frame,
      allowedOrigins: ["https://allowed.example"],
      resourceUri: "https://allowed.example/app",
      onMessage,
    });

    window.dispatchEvent(
      new MessageEvent("message", {
        data: {
          type: "refact:design-ready",
          payload: { resourceUri: "https://allowed.example/app" },
        },
        origin: "https://blocked.example",
        source: frameWindow,
      }),
    );

    expect(onMessage).not.toHaveBeenCalled();
    channel.dispose();
    frame.remove();
  });

  it("delivers one valid child Apply and rejects a malformed Apply", () => {
    const onMessage = vi.fn();
    const frame = document.createElement("iframe");
    document.body.append(frame);
    const frameWindow = frame.contentWindow;
    expect(frameWindow).not.toBeNull();
    const channel = createDesignChannel({
      frame,
      allowedOrigins: ["https://allowed.example"],
      resourceUri: "https://allowed.example/app",
      onMessage,
    });

    window.dispatchEvent(
      new MessageEvent("message", {
        data: {
          type: "refact:send-followup-turn",
          payload: { content: "Apply these edits" },
        },
        origin: "https://allowed.example",
        source: frameWindow,
      }),
    );
    window.dispatchEvent(
      new MessageEvent("message", {
        data: {
          type: "refact:send-followup-turn",
          payload: { content: null },
        },
        origin: "https://allowed.example",
        source: frameWindow,
      }),
    );

    expect(onMessage).toHaveBeenCalledTimes(1);
    expect(onMessage).toHaveBeenCalledWith({
      type: "refact:send-followup-turn",
      payload: { content: "Apply these edits" },
    });
    channel.dispose();
    frame.remove();
  });
});
