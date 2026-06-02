import { renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useCopyToClipboard } from "./useCopyToClipboard";

describe("useCopyToClipboard", () => {
  const originalClipboard = window.navigator.clipboard;
  const originalExecCommand = Object.getOwnPropertyDescriptor(
    document,
    "execCommand",
  );
  const execCommandMock = vi.fn(() => true);

  beforeEach(() => {
    vi.restoreAllMocks();
    execCommandMock.mockClear();
    Object.defineProperty(window.navigator, "clipboard", {
      configurable: true,
      value: originalClipboard,
    });
    Object.defineProperty(document, "execCommand", {
      configurable: true,
      value: execCommandMock,
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
    Object.defineProperty(window.navigator, "clipboard", {
      configurable: true,
      value: originalClipboard,
    });
    if (originalExecCommand) {
      Object.defineProperty(document, "execCommand", originalExecCommand);
    } else {
      delete (document as { execCommand?: Document["execCommand"] })
        .execCommand;
    }
  });

  it("uses navigator clipboard when available", () => {
    const writeText = vi.fn<(text: string) => Promise<void>>(() =>
      Promise.resolve(),
    );
    Object.defineProperty(window.navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    const { result } = renderHook(() => useCopyToClipboard());

    result.current("plan text");

    expect(writeText).toHaveBeenCalledWith("plan text");
    expect(execCommandMock).not.toHaveBeenCalled();
  });

  it("falls back when navigator clipboard is unavailable", () => {
    Object.defineProperty(window.navigator, "clipboard", {
      configurable: true,
      value: undefined,
    });
    const { result } = renderHook(() => useCopyToClipboard());

    result.current("fallback text");

    expect(execCommandMock).toHaveBeenCalledWith("copy");
  });

  it("falls back when navigator clipboard rejects", async () => {
    Object.defineProperty(window.navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: vi.fn<(text: string) => Promise<void>>(() =>
          Promise.reject(new Error("nope")),
        ),
      },
    });
    const { result } = renderHook(() => useCopyToClipboard());

    result.current("rejected text");

    await waitFor(() => {
      expect(execCommandMock).toHaveBeenCalledWith("copy");
    });
  });
});
