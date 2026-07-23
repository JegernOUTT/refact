import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  fireEvent,
  render,
  screen,
  waitFor,
} from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { setDockOpen } from "../workspaceSlice";
import { Dock } from "./Dock";

const originalMatchMedia = window.matchMedia;

function mockNarrow(narrow: boolean) {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: vi.fn((query: string): MediaQueryList => ({
      matches: narrow && query === "(max-width: 767px)",
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
}

describe("Dock", () => {
  afterEach(() => {
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: originalMatchMedia,
    });
    vi.restoreAllMocks();
  });

  it("renders capability sections and clamps persisted resize width", () => {
    mockNarrow(false);
    server.use(
      http.get("*/v1/files/tree", () =>
        HttpResponse.json({ path: "", entries: [], truncated: false }),
      ),
    );
    const view = render(<Dock />, {
      preloadedState: {
        config: {
          host: "web",
          lspPort: 8001,
          themeProps: { appearance: "dark" },
          capabilities: {
            filesPanel: true,
            gitPanel: false,
          },
        },
      },
    });
    const store = view.store;

    expect(screen.getByRole("radio", { name: "Files" })).toBeInTheDocument();
    expect(screen.queryByRole("radio", { name: "Git" })).toBeNull();
    expect(screen.getByRole("radio", { name: "Tasks" })).toBeInTheDocument();

    const dock = screen.getByTestId("workspace-dock");
    vi.spyOn(dock, "getBoundingClientRect").mockReturnValue({
      x: 0,
      y: 0,
      width: 280,
      height: 600,
      top: 0,
      right: 280,
      bottom: 600,
      left: 0,
      toJSON: () => ({}),
    });
    const splitter = screen.getByRole("separator", {
      name: "Resize workspace dock",
    });
    fireEvent.pointerDown(splitter, { button: 0, clientX: 280 });
    fireEvent.pointerMove(window, { clientX: 900 });
    fireEvent.pointerUp(window, { clientX: 900 });

    expect(store.getState().workspace.dock?.width).toBe(400);
  });

  it("uses a Sheet on narrow viewports and follows open state", async () => {
    mockNarrow(true);
    server.use(
      http.get("*/v1/files/tree", () =>
        HttpResponse.json({ path: "", entries: [], truncated: false }),
      ),
    );
    const view = render(<Dock />);

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    view.store.dispatch(setDockOpen(false));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("switches among enabled section placeholders", () => {
    mockNarrow(false);
    server.use(
      http.get("*/v1/files/tree", () =>
        HttpResponse.json({ path: "", entries: [], truncated: false }),
      ),
    );
    render(<Dock />);
    fireEvent.click(screen.getByRole("radio", { name: "Git" }));
    expect(screen.getByText("Git coming soon")).toBeInTheDocument();
  });
});
