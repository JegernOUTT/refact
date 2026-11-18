import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { server } from "../../utils/mockServer";
import { fireEvent, render, screen, waitFor } from "../../utils/test-utils";
import { framePosition } from "./projectBrowserFrame";
import { ProjectBrowserSurface } from "./ProjectBrowserSurface";

const frame = {
  runtime_id: "runtime-a",
  tab_id: "tab-a",
  mime: "image/png",
  data: "iVBORw0KGgo=",
  width: 600,
  height: 300,
  viewport: { width: 1200, height: 600 },
  url: "https://example.test/",
  title: "Example",
};

const liveStatus = {
  project_id: "test-project",
  running: true,
  connected: true,
  runtime_id: frame.runtime_id,
  active_tab: frame.tab_id,
  tabs: [{ tab_id: frame.tab_id, url: frame.url, title: frame.title }],
  migration: { state: "resolved" },
};

describe("Project Browser surface", () => {
  it("maps displayed screenshot pixels to CSS viewport coordinates and rejects letterboxing", () => {
    const bounds = { left: 10, top: 20, width: 300, height: 200 };
    expect(framePosition(frame, bounds, 160, 120)).toEqual({ x: 600, y: 300 });
    expect(framePosition(frame, bounds, 160, 25)).toBeNull();
  });

  it("never starts or migrates before the user chooses, and disables in-use profiles", async () => {
    const commands: string[] = [];
    let resolved = false;
    server.use(
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json(
          resolved
            ? liveStatus
            : {
                project_id: "test-project",
                running: false,
                connected: false,
                tabs: [],
                migration: { state: "available" },
              },
        ),
      ),
      http.post("*/v1/browser/project/migration/candidates", () =>
        HttpResponse.json({
          state: "available",
          candidates: [
            { legacy_chat_id: "moving", last_used_ms: 10, locked: false },
            { legacy_chat_id: "in-use", last_used_ms: null, locked: true },
          ],
        }),
      ),
      http.post("*/v1/browser/project/migration/adopt", async ({ request }) => {
        expect(await request.json()).toEqual({
          legacy_chat_id: "moving",
          confirm: true,
        });
        commands.push("adopt");
        resolved = true;
        return HttpResponse.json({ state: "resolved" });
      }),
      http.post("*/v1/browser/project/start", () => {
        commands.push("start");
        return HttpResponse.json(liveStatus);
      }),
      http.post("*/v1/browser/project/frame", () => HttpResponse.json(frame)),
    );

    render(<ProjectBrowserSurface />);
    const chooser = await screen.findByRole("combobox", {
      name: "Chat browser profile",
    });
    const adopt = screen.getByRole("button", { name: "Adopt profile" });
    expect(commands).toEqual([]);
    expect(adopt).toBeDisabled();
    expect(
      screen.getByRole("option", { name: "in-use (in use)" }),
    ).toBeDisabled();
    fireEvent.change(chooser, { target: { value: "moving" } });
    fireEvent.click(adopt);
    await screen.findByRole("textbox", { name: "Address" });
    expect(commands).toEqual(["adopt", "start"]);
  });

  it("sends ordered key and paste input with the displayed frame fence", async () => {
    const inputs: unknown[] = [];
    server.use(
      http.post("*/v1/browser/project/status", () =>
        HttpResponse.json(liveStatus),
      ),
      http.post("*/v1/browser/project/frame", () => HttpResponse.json(frame)),
      http.post("*/v1/browser/project/input", async ({ request }) => {
        inputs.push(await request.json());
        return HttpResponse.json({ ok: true });
      }),
    );
    render(<ProjectBrowserSurface />);
    const image = await screen.findByRole("img", { name: "Browser page" });
    fireEvent.keyDown(image, { key: "Backspace" });
    fireEvent.paste(image, { clipboardData: { getData: () => "hello" } });
    await waitFor(() =>
      expect(inputs).toEqual([
        {
          runtime_id: "runtime-a",
          tab_id: "tab-a",
          kind: "key",
          key: "Backspace",
          modifiers: [],
        },
        {
          runtime_id: "runtime-a",
          tab_id: "tab-a",
          kind: "text",
          text: "hello",
        },
      ]),
    );
  });
});
