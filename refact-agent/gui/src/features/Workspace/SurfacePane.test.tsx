import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { render, screen } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import * as ChatModule from "../Chat/Chat";
import { SurfacePane } from "./SurfacePane";
import { makeSurfaceKey, parseSurfaceKey } from "./surfaceKey";

vi.spyOn(ChatModule, "Chat").mockImplementation(({ chatId }) => (
  <section data-testid="chat-surface" data-chat-id={chatId ?? ""}>
    Chat surface {chatId ?? ""}
  </section>
));

describe("SurfacePane", () => {
  beforeEach(() => {
    server.use(
      http.get("*/v1/files/tree", () =>
        HttpResponse.json({ path: "", entries: [], truncated: false }),
      ),
    );
  });

  it("renders an empty placeholder when no surface is selected", () => {
    render(<SurfacePane surfaceKey={null} />);

    expect(screen.getByText("No surface selected")).toBeInTheDocument();
    expect(
      screen.getByText("Open or drag a workspace tab into this pane."),
    ).toBeInTheDocument();
  });

  it("renders a chat surface for chat surface keys", () => {
    const surfaceKey = makeSurfaceKey("chat", "chat-a");

    expect(parseSurfaceKey(surfaceKey)).toEqual({ kind: "chat", id: "chat-a" });

    render(<SurfacePane surfaceKey={surfaceKey} />);

    expect(screen.getByTestId("chat-surface")).toHaveAttribute(
      "data-chat-id",
      "chat-a",
    );
    expect(
      document.querySelector(`[data-surface-key="${surfaceKey}"]`),
    ).toBeInTheDocument();
    expect(screen.queryByText("No surface selected")).not.toBeInTheDocument();
  });

  it("renders the registered files panel surface", () => {
    const surfaceKey = makeSurfaceKey("files", "main");

    render(<SurfacePane surfaceKey={surfaceKey} />);

    expect(screen.getByText("Explorer")).toBeInTheDocument();
    expect(screen.getByText("Select a file")).toBeInTheDocument();
    expect(
      document.querySelector(`[data-surface-key="${surfaceKey}"]`),
    ).toBeInTheDocument();
  });

  it("renders nothing for non-chat surface keys", () => {
    const surfaceKey = makeSurfaceKey("task", "task-a");

    expect(parseSurfaceKey(surfaceKey)).toEqual({ kind: "task", id: "task-a" });

    const { container } = render(<SurfacePane surfaceKey={surfaceKey} />);

    expect(container.firstElementChild).toBeEmptyDOMElement();
    expect(screen.queryByTestId("chat-surface")).not.toBeInTheDocument();
    expect(screen.queryByText("No surface selected")).not.toBeInTheDocument();
  });
});
