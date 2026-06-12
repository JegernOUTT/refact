import { beforeAll, afterEach, afterAll, vi } from "vitest";
import {
  stubResizeObserver,
  cleanup,
  stubIntersectionObserver,
} from "./test-utils";
import MatchMediaMock from "vitest-matchmedia-mock";
import React from "react";
const matchMediaMock = new MatchMediaMock();

type VirtuosoMockProps = {
  data?: unknown[];
  itemContent: (index: number, item: unknown) => React.ReactNode;
  components?: {
    Header?: React.ComponentType;
    Scroller?: React.ComponentType<React.HTMLAttributes<HTMLDivElement>>;
    List?: React.ComponentType<React.HTMLAttributes<HTMLDivElement>>;
    Item?: React.ComponentType<React.HTMLAttributes<HTMLDivElement>>;
    Footer?: React.ComponentType;
  };
};

(globalThis as Record<string, unknown>).__VIRTUOSO_CALLS__ = [];

vi.mock("react-virtuoso", async () => {
  const ReactModule = await vi.importActual<typeof import("react")>("react");

  return {
    Virtuoso: ReactModule.forwardRef<HTMLDivElement, VirtuosoMockProps>(
      ({ data, itemContent, components, ...props }, _ref) => {
        const calls =
          ((globalThis as Record<string, unknown>).__VIRTUOSO_CALLS__ as
            | unknown[]
            | undefined) ?? [];
        calls.push(props);
        (globalThis as Record<string, unknown>).__VIRTUOSO_CALLS__ = calls;
        const header = components?.Header
          ? ReactModule.createElement(components.Header)
          : null;
        const list = ReactModule.createElement(
          components?.List ?? "div",
          null,
          header,
          ...(data ?? []).map((item, i) => itemContent(i, item)),
          components?.Footer
            ? ReactModule.createElement(components.Footer)
            : null,
        );

        return ReactModule.createElement(
          components?.Scroller ?? "div",
          null,
          list,
        );
      },
    ),
    VirtuosoGrid: ReactModule.forwardRef<HTMLDivElement, VirtuosoMockProps>(
      ({ data, itemContent, components, ...props }, _ref) => {
        const calls =
          ((globalThis as Record<string, unknown>).__VIRTUOSO_CALLS__ as
            | unknown[]
            | undefined) ?? [];
        calls.push(props);
        (globalThis as Record<string, unknown>).__VIRTUOSO_CALLS__ = calls;
        return ReactModule.createElement(
          components?.List ?? "div",
          null,
          ...(data ?? []).map((item, i) =>
            ReactModule.createElement(
              components?.Item ?? "div",
              { key: i },
              itemContent(i, item),
            ),
          ),
        );
      },
    ),
  };
});

(globalThis as Record<string, unknown>).__REFACT_LSP_PORT__ = 8001;

beforeAll(() => {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    x: 0,
    y: 0,
    width: 1024,
    height: 768,
    top: 0,
    right: 1024,
    bottom: 768,
    left: 0,
    toJSON: () => ({}),
  });
  stubResizeObserver();
  stubIntersectionObserver();
  Element.prototype.scrollIntoView = vi.fn();

  // Mock localStorage for tests
  const storage = new Map<string, string>();
  const localStorageMock: Storage = {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => {
      storage.set(key, value);
    },
    removeItem: (key: string) => {
      storage.delete(key);
    },
    clear: () => {
      storage.clear();
    },
    key: (index: number) => Array.from(storage.keys())[index] ?? null,
    get length() {
      return storage.size;
    },
  };
  global.localStorage = localStorageMock;
});

afterEach(() => {
  cleanup();
});

afterAll(() => {
  matchMediaMock.destroy();
});

vi.mock("lottie-react", () => {
  return {
    default: vi.fn(),
    useLottie: vi.fn(() => {
      return {
        View: React.createElement("div"),
        playSegments: vi.fn(),
      };
    }),
  };
});
