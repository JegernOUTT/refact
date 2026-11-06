import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import { render, screen } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { setBackendStatus } from "../Connection/connectionSlice";
import { setHistoryLoading } from "../History/historySlice";
import { sidebarSectionSnapshotReceived } from "../Sidebar/sidebarSlice";
import { Dashboard } from "./Dashboard";
import type { StreamSectionProps } from "./components/Stream/StreamSection";

vi.mock("./components/Stream", () => ({
  StreamSection: ({ filter }: StreamSectionProps) => (
    <div data-testid="main-stream">
      {filter.kind}:{filter.query}
    </div>
  ),
}));

vi.mock("./components/HeroBlock/HeroBlock", () => ({
  HeroBlock: () => <div>Removed hero</div>,
}));

vi.mock("./components/ContinueSection/ContinueSection", () => ({
  ContinueSection: () => <div>Removed continue cards</div>,
}));

vi.mock("./components/AttentionBand/AttentionBand", () => ({
  AttentionBand: () => <div>Removed attention band</div>,
}));

describe("Dashboard composition", () => {
  beforeEach(() => {
    server.use(http.get("*/v1/tasks", () => HttpResponse.json([])));
  });

  it("keeps filters and the main stream without the removed home sections", async () => {
    const { store, user, container } = render(<Dashboard />);
    act(() => {
      store.dispatch(setBackendStatus({ status: "online" }));
      store.dispatch(setHistoryLoading(false));
      store.dispatch(
        sidebarSectionSnapshotReceived({ section: "chats", status: "ready" }),
      );
      store.dispatch(
        sidebarSectionSnapshotReceived({ section: "tasks", status: "ready" }),
      );
    });

    expect(screen.getByTestId("main-stream")).toHaveTextContent("all:");
    expect(screen.getByRole("searchbox")).toBeInTheDocument();
    expect(screen.queryByText(/Removed/)).not.toBeInTheDocument();
    expect(container.querySelectorAll(".rf-stagger > *")).toHaveLength(2);
    await user.click(screen.getByRole("radio", { name: /^Tasks/ }));
    expect(screen.getByTestId("main-stream")).toHaveTextContent("task:");
  });

  it("preserves offline, loading and error states instead of rendering the stream", () => {
    const { store } = render(<Dashboard />);
    expect(screen.getByText("Connecting to Refact…")).toBeInTheDocument();
    expect(screen.queryByRole("searchbox")).not.toBeInTheDocument();
    act(() => {
      store.dispatch(setBackendStatus({ status: "offline" }));
    });
    expect(screen.getByText("Refact engine unavailable")).toBeInTheDocument();
    act(() => {
      store.dispatch(setBackendStatus({ status: "online" }));
    });
    expect(screen.getByRole("searchbox")).toBeInTheDocument();
    expect(screen.getByText("Loading")).toBeInTheDocument();
    expect(screen.queryByTestId("main-stream")).not.toBeInTheDocument();
    act(() => {
      store.dispatch(
        sidebarSectionSnapshotReceived({
          section: "tasks",
          status: "error",
          error: "Task service unavailable",
        }),
      );
    });
    expect(
      screen.getByText("Failed to load your workspace"),
    ).toBeInTheDocument();
    expect(screen.getByText("Task service unavailable")).toBeInTheDocument();
    expect(screen.queryByTestId("main-stream")).not.toBeInTheDocument();
  });
});
