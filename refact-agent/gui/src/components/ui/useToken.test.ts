import { cleanup, renderHook, waitFor } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useToken, useTokens } from "./useToken";

const styleId = "use-token-test-style";

function installTokenStyles(): void {
  const style = document.createElement("style");
  style.id = styleId;
  style.textContent = `
    :root { --rf-test-direct: root-value; --rf-test-mode: dark-value; --rf-test-a: value-a; --rf-test-b: value-b; }
    .light { --rf-test-mode: light-value; }
  `;
  document.head.append(style);
}

function resetDocument(): void {
  document.documentElement.className = "";
  document.documentElement.removeAttribute("data-appearance");
  document.documentElement.removeAttribute("data-host");
  document.body.className = "";
  document.body.removeAttribute("data-appearance");
  document.body.removeAttribute("data-host");
  document.documentElement.removeAttribute("style");
  document.getElementById(styleId)?.remove();
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  resetDocument();
});

describe("useToken", () => {
  it("reads a root custom property with or without a leading dash", () => {
    installTokenStyles();

    const dashed = renderHook(() => useToken("--rf-test-direct"));
    const bare = renderHook(() => useToken("rf-test-direct"));

    expect(dashed.result.current).toBe("root-value");
    expect(bare.result.current).toBe("root-value");
  });

  it("updates after appearance class changes", async () => {
    installTokenStyles();
    document.documentElement.classList.add("dark");

    const { result } = renderHook(() => useToken("--rf-test-mode"));

    expect(result.current).toBe("dark-value");

    act(() => {
      document.documentElement.classList.remove("dark");
      document.documentElement.classList.add("light");
    });

    await waitFor(() => expect(result.current).toBe("light-value"));
  });

  it("reads multiple tokens from one hook", () => {
    installTokenStyles();

    const names = ["--rf-test-a", "rf-test-b"];
    const { result } = renderHook(() => useTokens(names));

    expect(result.current).toEqual({
      "--rf-test-a": "value-a",
      "rf-test-b": "value-b",
    });
  });

  it("cleans up observers on unmount", () => {
    const disconnect = vi.fn();
    const observe = vi.fn();

    class MockMutationObserver {
      observe = observe;
      disconnect = disconnect;
    }

    vi.stubGlobal("MutationObserver", MockMutationObserver);

    const { unmount } = renderHook(() => useToken("--rf-test-direct"));

    expect(observe).toHaveBeenCalled();

    unmount();

    expect(disconnect).toHaveBeenCalledTimes(1);
  });
});
