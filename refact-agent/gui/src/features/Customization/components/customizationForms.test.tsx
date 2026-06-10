import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";

import { fireEvent, render, screen } from "../../../utils/test-utils";
import {
  chatSessionCommand,
  goodCaps,
  goodPing,
  server,
} from "../../../utils/mockServer";
import { ModeForm } from "./ModeForm";
import { ToolboxCommandForm } from "./ToolboxCommandForm";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "web" as const,
    engineServed: true,
  },
};

function setupModeHandlers() {
  server.use(
    goodCaps,
    goodPing,
    chatSessionCommand,
    http.get("*/v1/chat-modes", () =>
      HttpResponse.json({ modes: [], errors: [] }),
    ),
  );
}

describe("Customization form numeric fields", () => {
  it("does not patch NaN for toolbox selection range inputs", () => {
    const onPatch = vi.fn();
    render(
      <ToolboxCommandForm
        config={{ selection_needed: [1, 100], messages: [] }}
        onPatch={onPatch}
      />,
    );

    fireEvent.change(screen.getByDisplayValue("1"), {
      target: { value: "abc" },
    });

    expect(onPatch).not.toHaveBeenCalled();
  });

  it("clears mode UI order instead of patching NaN", async () => {
    setupModeHandlers();
    const onPatch = vi.fn();
    const { user } = render(
      <ModeForm config={{ ui: { order: 3 } }} onPatch={onPatch} />,
      { preloadedState: CONFIG_STATE },
    );

    await user.click(screen.getByRole("button", { name: "Advanced" }));
    fireEvent.change(screen.getByDisplayValue("3"), {
      target: { value: "abc" },
    });

    expect(onPatch).toHaveBeenLastCalledWith({
      path: ["ui", "order"],
      value: undefined,
    });
    expect(onPatch).not.toHaveBeenCalledWith({
      path: ["ui", "order"],
      value: NaN,
    });
  });
});
