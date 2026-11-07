import { render, screen, fireEvent, waitFor } from "../utils/test-utils";
import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { server } from "../utils/mockServer";
import { ExtItemList } from "../features/Extensions/components/ExtItemList";
import { SkillEditor } from "../features/Extensions/components/SkillEditor";
import { MarketplacePanel } from "../features/Extensions/components/MarketplacePanel";
import { Extensions } from "../features/Extensions/Extensions";
import type { SkillRegistryItem } from "../services/refact/extensions";

const MOCK_ITEMS: SkillRegistryItem[] = [
  {
    name: "my_skill",
    description: "A global skill",
    source: "global",
    source_label: "Global",
    scope: "global",
    read_only: false,
    file_path: "/home/.config/refact/skills/my_skill/SKILL.md",
  },
  {
    name: "local_skill",
    description: "A local project skill",
    source: "local",
    source_label: "Local",
    scope: "local",
    read_only: false,
    file_path: "/project/.refact/skills/local_skill/SKILL.md",
  },
  {
    name: "plugin_skill",
    description: "A plugin skill",
    source: "plugin:my-plugin",
    source_label: "my-plugin",
    scope: "plugin",
    read_only: true,
    file_path:
      "/home/.config/refact/plugins/installed/my-plugin/skills/plugin_skill/SKILL.md",
  },
];

const MARKETPLACE_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

describe("ExtItemList", () => {
  it("shows delete button only for non-read-only items", () => {
    render(
      <ExtItemList
        items={MOCK_ITEMS}
        selectedId={null}
        onSelect={() => undefined}
        onCreate={() => undefined}
        onDelete={() => undefined}
      />,
    );

    expect(screen.getByLabelText("Delete my_skill")).toBeDefined();
    expect(screen.getByLabelText("Delete local_skill")).toBeDefined();
    expect(screen.queryByLabelText("Delete plugin_skill")).toBeNull();
  });

  it("calls onDelete with name and scope when delete button clicked", () => {
    const onDelete = vi.fn();
    render(
      <ExtItemList
        items={MOCK_ITEMS}
        selectedId={null}
        onSelect={() => undefined}
        onCreate={() => undefined}
        onDelete={onDelete}
      />,
    );
    const deleteBtn = screen.getByLabelText("Delete local_skill");
    fireEvent.click(deleteBtn);
    expect(onDelete).toHaveBeenCalledWith("local_skill", "local");
  });
});

describe("MarketplacePanel", () => {
  it("searches plugins with null descriptions and matches installed state by marketplace and name", async () => {
    const deleteRequest = vi.fn();
    server.use(
      http.get("*/v1/plugins/marketplaces", () =>
        HttpResponse.json({
          marketplaces: [
            {
              name: "alpha",
              source: "owner/alpha",
              added_at: "2024-01-01T00:00:00Z",
            },
            {
              name: "beta",
              source: "owner/beta",
              added_at: "2024-01-01T00:00:00Z",
            },
          ],
        }),
      ),
      http.get("*/v1/plugins/installed", () =>
        HttpResponse.json({
          installed: [
            {
              name: "duplicate-plugin",
              marketplace: "alpha",
              install_dir: "/plugins/duplicate-plugin",
              installed_at: "2024-01-01T00:00:00Z",
            },
          ],
        }),
      ),
      http.get("*/v1/plugins/marketplace/alpha/plugins", () =>
        HttpResponse.json({
          plugins: [
            {
              name: "duplicate-plugin",
              description: null,
              marketplace: "alpha",
            },
          ],
        }),
      ),
      http.get("*/v1/plugins/marketplace/beta/plugins", () =>
        HttpResponse.json({
          plugins: [
            {
              name: "duplicate-plugin",
              description: "Beta marketplace copy",
              marketplace: "beta",
            },
          ],
        }),
      ),
      http.delete("*/v1/plugins/installed/duplicate-plugin", () => {
        deleteRequest();
        return HttpResponse.json({ deleted: true });
      }),
    );

    const { user } = render(<MarketplacePanel />, {
      preloadedState: MARKETPLACE_STATE,
    });

    await screen.findByText("Beta marketplace copy");
    expect(screen.getByRole("button", { name: /^Install$/i })).toBeDefined();
    expect(screen.getAllByText("Installed")).toHaveLength(1);

    await user.type(
      screen.getByPlaceholderText("Search plugins…"),
      "duplicate",
    );

    await waitFor(() => {
      expect(screen.getByText("Beta marketplace copy")).toBeDefined();
    });
    expect(screen.getByRole("button", { name: /^Install$/i })).toBeDefined();

    await user.click(screen.getAllByRole("button", { name: /uninstall/i })[0]);

    await waitFor(() => {
      expect(deleteRequest).toHaveBeenCalledOnce();
    });
  });
});

describe("SkillEditor", () => {
  it("renders form fields reflecting loaded skill data", async () => {
    server.use(
      http.get("*/v1/ext/skills/my_skill", () => {
        return HttpResponse.json({
          name: "my_skill",
          description: "A test skill",
          user_invocable: true,
          allowed_tools: ["shell"],
          model: null,
          context: null,
          agent: null,
          argument_hint: "[arg]",
          body: "# My Skill\nDo something.",
          raw_content:
            "---\ndescription: A test skill\n---\n# My Skill\nDo something.",
          source: "global",
          file_path: "/home/.config/refact/skills/my_skill/SKILL.md",
        });
      }),
    );

    render(<SkillEditor name="my_skill" onBack={() => undefined} />, {
      preloadedState: {
        config: {
          apiKey: "test",
          lspPort: 8001,
          themeProps: {},
          host: "vscode",
        },
      },
    });

    const nameInput = await screen.findByDisplayValue("my_skill");
    expect(nameInput).toBeDefined();

    const description = await screen.findByDisplayValue("A test skill");
    expect(description).toBeDefined();
  });
});

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

describe("Extensions", () => {
  it("shows error state when registry fails to load", async () => {
    server.use(
      http.get("*/v1/ext/registry", () => {
        return new HttpResponse(null, { status: 500 });
      }),
    );

    render(
      <Extensions
        host="vscode"
        tabbed={false}
        backFromExtensions={() => undefined}
      />,
      { preloadedState: CONFIG_STATE },
    );

    const errorMsg = await screen.findByText(
      "Failed to load extensions registry",
    );
    expect(errorMsg).toBeDefined();
    expect(screen.getByText("Retry")).toBeDefined();
  });

  it("shows delete confirmation dialog and can be cancelled", async () => {
    server.use(
      http.get("*/v1/ext/registry", () => {
        return HttpResponse.json({
          skills: [
            {
              name: "my_skill",
              description: "A global skill",
              source: "global",
              source_label: "Global",
              scope: "global",
              read_only: false,
              file_path: "/home/.config/refact/skills/my_skill/SKILL.md",
            },
          ],
          slash_commands: [],
          hooks: [],
          has_project_root: true,
        });
      }),
    );

    render(
      <Extensions
        host="vscode"
        tabbed={false}
        backFromExtensions={() => undefined}
      />,
      { preloadedState: CONFIG_STATE },
    );

    const deleteBtn = await screen.findByLabelText("Delete my_skill");
    fireEvent.click(deleteBtn);

    const confirmTitle = await screen.findByText("Delete extension?");
    expect(confirmTitle).toBeDefined();
    const cancelBtn = screen.getByText("Cancel");
    expect(cancelBtn).toBeDefined();

    fireEvent.click(cancelBtn);

    await waitFor(() => {
      expect(screen.queryByText("Delete extension?")).toBeNull();
    });
  });
});
