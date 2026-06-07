import { cleanup, renderHook, waitFor } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useToken, useTokens } from "./useToken";

const styleId = "use-token-test-style";
const darkSchemeQuery = "(prefers-color-scheme: dark)";
const lightSchemeQuery = "(prefers-color-scheme: light)";

function installTokenStyles(): void {
  const style = document.createElement("style");
  style.id = styleId;
  style.textContent = `
    :root { --rf-test-direct: root-value; --rf-test-mode: dark-value; --rf-test-a: value-a; --rf-test-b: value-b; --rf-test-host: default-host; }
    .light { --rf-test-mode: light-value; }
  `;
  document.head.append(style);
}

function createColorSchemeMatchMediaMock(): {
  matchMedia: (query: string) => MediaQueryList;
  setDarkScheme: (enabled: boolean) => void;
} {
  let darkScheme = false;
  const listeners = new Map<string, Set<(event: MediaQueryListEvent) => void>>();

  const getListeners = (query: string) => {
    const existing = listeners.get(query);
    if (existing) return existing;

    const next = new Set<(event: MediaQueryListEvent) => void>();
    listeners.set(query, next);
    return next;
  };

  const matchesQuery = (query: string) => {
    if (query === darkSchemeQuery) return darkScheme;
    if (query === lightSchemeQuery) return !darkScheme;
    return false;
  };

  const matchMedia = vi.fn((query: string): MediaQueryList => {
    return {
      media: query,
      get matches() {
        return matchesQuery(query);
      },
      onchange: null,
      addEventListener: (
        _type: string,
        listener: EventListenerOrEventListenerObject,
      ) => {
        if (typeof listener === "function") {
          getListeners(query).add(listener as (event: MediaQueryListEvent) => void);
        }
      },
      removeEventListener: (
        _type: string,
        listener: EventListenerOrEventListenerObject,
      ) => {
        if (typeof listener === "function") {
          getListeners(query).delete(
            listener as (event: MediaQueryListEvent) => void,
          );
        }
      },
      addListener: (listener) => {
        if (listener) {
          getListeners(query).add(listener);
        }
      },
      removeListener: (listener) => {
        if (listener) {
          getListeners(query).delete(listener);
        }
      },
      dispatchEvent: () => true,
    };
  });

  const setDarkScheme = (enabled: boolean) => {
    darkScheme = enabled;
    [darkSchemeQuery, lightSchemeQuery].forEach((query) => {
      const event = {
        matches: matchesQuery(query),
        media: query,
      } as MediaQueryListEvent;
      getListeners(query).forEach((listener) => listener(event));
    });
  };

  return { matchMedia, setDarkScheme };
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

beforeEach(() => {
  resetDocument();
});

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

  it("updates after body class changes", async () => {
    installTokenStyles();

    const { result } = renderHook(() => useToken("--rf-test-direct"));

    expect(result.current).toBe("root-value");

    act(() => {
      document.documentElement.style.setProperty(
        "--rf-test-direct",
        "body-triggered-value",
      );
      document.body.classList.add("light");
    });

    await waitFor(() => expect(result.current).toBe("body-triggered-value"));
  });

  it("updates after body data-appearance changes", async () => {
    installTokenStyles();

    const { result } = renderHook(() => useToken("--rf-test-direct"));

    expect(result.current).toBe("root-value");

    act(() => {
      document.documentElement.style.setProperty(
        "--rf-test-direct",
        "body-appearance-value",
      );
      document.body.dataset.appearance = "dark";
    });

    await waitFor(() => expect(result.current).toBe("body-appearance-value"));
  });

  it("updates after documentElement data-host changes", async () => {
    installTokenStyles();

    const { result } = renderHook(() => useToken("--rf-test-host"));

    expect(result.current).toBe("default-host");

    act(() => {
      document.documentElement.style.setProperty(
        "--rf-test-host",
        "host-triggered-value",
      );
      document.documentElement.dataset.host = "jetbrains";
    });

    await waitFor(() => expect(result.current).toBe("host-triggered-value"));
  });

  it("updates after a color scheme media query change", async () => {
    installTokenStyles();
    const mock = createColorSchemeMatchMediaMock();
    vi.stubGlobal("matchMedia", mock.matchMedia);

    const { result } = renderHook(() => useToken("--rf-test-direct"));

    expect(result.current).toBe("root-value");

    act(() => {
      document.documentElement.style.setProperty(
        "--rf-test-direct",
        "media-triggered-value",
      );
      mock.setDarkScheme(true);
    });

    await waitFor(() => expect(result.current).toBe("media-triggered-value"));
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
