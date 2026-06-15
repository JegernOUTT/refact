import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";

import { fireEvent, render, screen, waitFor } from "../../utils/test-utils";
import {
  chatSessionCommand,
  goodCaps,
  goodPing,
  server,
} from "../../utils/mockServer";
import { Customization } from "./Customization";
import type { ConfigItem } from "../../services/refact/customization";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "web" as const,
    engineServed: true,
  },
};

function configItem(
  kind: string,
  id: string,
  title: string,
  scope: "global" | "local" = "global",
): ConfigItem {
  return {
    id,
    kind,
    title,
    file_path: `/tmp/${id}.yaml`,
    specific: false,
    scope,
    global_path: `/tmp/global/${id}.yaml`,
    local_path: `/tmp/local/${id}.yaml`,
    global_exists: scope === "global",
    local_exists: scope === "local",
  };
}

const registry = {
  modes: [
    configItem("modes", "agent_mode_with_a_long_identifier", "Agent Mode"),
  ],
  subagents: [
    configItem("subagents", "review_subagent_with_a_long_identifier", "Review"),
    configItem("subagents", "planner_subagent", "Planner"),
  ],
  toolbox_commands: [
    configItem("toolbox_commands", "summarize_command", "Summarize"),
  ],
  code_lens: [configItem("code_lens", "explain_code_lens", "Explain Code")],
  errors: [],
  has_project_root: true,
};

function modeDetail(id: string) {
  return {
    config: {
      schema_version: 1,
      id,
      title: "Agent Mode",
      description: "Mode for testing",
      prompt: "Prompt",
    },
    file_path: `/tmp/${id}.yaml`,
    raw_yaml: `id: ${id}\ntitle: Agent Mode\nprompt: Prompt\n`,
    scope: "global",
  };
}

function setupHandlers() {
  server.use(
    goodCaps,
    http.get("*/v1/customization/registry", () => HttpResponse.json(registry)),
    goodPing,
    chatSessionCommand,
    http.get("*/v1/chat-modes", () =>
      HttpResponse.json({ modes: [], errors: [] }),
    ),
    http.get("*/v1/customization/modes/:id", ({ params }) =>
      HttpResponse.json(modeDetail(String(params.id))),
    ),
  );
}

function setupSaveHandler(onSave: (body: unknown) => void) {
  setupHandlers();
  server.use(
    http.put("*/v1/customization/modes/:id", async ({ request }) => {
      onSave(await request.json());
      return HttpResponse.json({
        ok: true,
        file_path: "/tmp/saved.yaml",
        scope: "global",
        errors: [],
      });
    }),
  );
}

function setupDeleteHandler(onDelete: () => void) {
  setupHandlers();
  server.use(
    http.delete("*/v1/customization/modes/:id", () => {
      onDelete();
      return HttpResponse.json({ ok: true, scope: "global", errors: [] });
    }),
  );
}

describe("Customization", () => {
  it("renders the canonical tabs with preserved count badges", async () => {
    setupHandlers();

    render(
      <Customization
        embedded
        host="web"
        tabbed={false}
        backFromCustomization={vi.fn()}
      />,
      { preloadedState: CONFIG_STATE },
    );

    await screen.findByRole("heading", { name: "Customization" });

    expect(screen.getByRole("tab", { name: /Modes\s*1/i })).toBeInTheDocument();
    expect(
      screen.getByRole("tab", { name: /Subagents\s*2/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("tab", { name: /Toolbox\s*1/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("tab", { name: /Code Lens\s*1/i }),
    ).toBeInTheDocument();
  });

  it("uses accessible segmented controls for editor and scope toggles", async () => {
    setupHandlers();

    render(
      <Customization
        embedded
        host="web"
        tabbed={false}
        initialKind="modes"
        initialConfigId="agent_mode_with_a_long_identifier"
        backFromCustomization={vi.fn()}
      />,
      { preloadedState: CONFIG_STATE },
    );

    expect(
      await screen.findByText("agent_mode_with_a_long_identifier"),
    ).toBeInTheDocument();
    expect(
      await screen.findByRole("radiogroup", { name: "Editor view" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Form editor" })).toBeChecked();
    expect(
      screen.getByRole("radio", { name: "YAML editor" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("radiogroup", { name: "Save scope" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Global scope" })).toBeChecked();
    expect(
      screen.getByRole("radio", { name: "Project scope" }),
    ).toBeInTheDocument();
  });

  it("saves the latest YAML edit when Save is clicked immediately", async () => {
    let savedBody: unknown;
    setupSaveHandler((body) => {
      savedBody = body;
    });

    const { container, user } = render(
      <Customization
        embedded
        host="web"
        tabbed={false}
        initialKind="modes"
        initialConfigId="agent_mode_with_a_long_identifier"
        backFromCustomization={vi.fn()}
      />,
      { preloadedState: CONFIG_STATE },
    );

    expect(
      await screen.findByText("agent_mode_with_a_long_identifier"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "YAML editor" }));
    const textarea = container.querySelector("textarea");
    expect(textarea).not.toBeNull();
    if (!textarea) throw new Error("expected YAML textarea");

    fireEvent.change(textarea, {
      target: {
        value:
          "id: agent_mode_with_a_long_identifier\ntitle: Agent Mode\nprompt: Updated Prompt\n",
      },
    });
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(savedBody).toMatchObject({
        config: { prompt: "Updated Prompt" },
      });
    });
  });

  it("uses an in-app confirmation before deleting a config", async () => {
    const confirmSpy = vi.spyOn(window, "confirm");
    const onDelete = vi.fn();
    setupDeleteHandler(onDelete);

    const { user } = render(
      <Customization
        embedded
        host="web"
        tabbed={false}
        backFromCustomization={vi.fn()}
      />,
      { preloadedState: CONFIG_STATE },
    );

    await screen.findByRole("heading", { name: "Customization" });
    await user.click(
      screen.getByLabelText("Delete agent_mode_with_a_long_identifier"),
    );

    expect(confirmSpy).not.toHaveBeenCalled();
    expect(onDelete).not.toHaveBeenCalled();
    expect(
      await screen.findByRole("heading", { name: "Delete configuration?" }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(onDelete).toHaveBeenCalledOnce());
    confirmSpy.mockRestore();
  });
});
