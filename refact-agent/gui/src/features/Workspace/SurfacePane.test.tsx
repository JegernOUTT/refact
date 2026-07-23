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
    server.use(
      http.get("*/v1/files/read", ({ request }) => {
        const path = new URL(request.url).searchParams.get("path") ?? "";
        return HttpResponse.json({
          path,
          content: "const value = 1;",
          language: "typescript",
          size: 16,
          truncated: false,
          line_start: null,
          line_end: null,
          mtime_ms: 1,
        });
      }),
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

  it("renders a file viewer surface", async () => {
    const surfaceKey = makeSurfaceKey("file", "/workspace/main.ts");

    render(<SurfacePane surfaceKey={surfaceKey} />);

    expect(await screen.findByText("const value = 1;")).toBeInTheDocument();
    expect(screen.getByLabelText("File viewer")).toBeInTheDocument();
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
