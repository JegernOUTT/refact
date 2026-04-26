import { render, screen, fireEvent, waitFor } from "../utils/test-utils";
import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { server } from "../utils/mockServer";
import { ExtItemList } from "../features/Extensions/components/ExtItemList";
import { SkillEditor } from "../features/Extensions/components/SkillEditor";
import { MarketplacePluginCard } from "../features/Extensions/components/MarketplacePluginCard";
import { Extensions } from "../features/Extensions/Extensions";
import type { SkillRegistryItem } from "../services/refact/extensions";
import type { PluginEntry } from "../services/refact/plugins";

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

describe("ExtItemList", () => {
  it("renders items with correct source badges", () => {
    render(
      <ExtItemList
        items={MOCK_ITEMS}
        selectedId={null}
        onSelect={() => undefined}
        onCreate={() => undefined}
        onDelete={() => undefined}
      />,
    );

    expect(screen.getByText("my_skill")).toBeDefined();
    expect(screen.getByText("local_skill")).toBeDefined();
    expect(screen.getByText("plugin_skill")).toBeDefined();

    expect(screen.getByText("Global")).toBeDefined();
    expect(screen.getByText("Local")).toBeDefined();
    expect(screen.getByText("Plugin")).toBeDefined();
  });

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

  it("marks selected item", () => {
    const { container } = render(
      <ExtItemList
        items={MOCK_ITEMS}
        selectedId="my_skill"
        onSelect={() => undefined}
        onCreate={() => undefined}
        onDelete={() => undefined}
      />,
    );

    const selectedEl = container.querySelector(
      '[aria-label="Select my_skill"]',
    );
    expect(selectedEl?.className).toContain("selected");
  });

  it("renders empty state when no items", () => {
    render(
      <ExtItemList
        items={[]}
        selectedId={null}
        onSelect={() => undefined}
        onCreate={() => undefined}
        onDelete={() => undefined}
      />,
    );
    expect(screen.getByText("No items found")).toBeDefined();
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

describe("MarketplacePluginCard", () => {
  const ENGINE_PLUGIN: PluginEntry = {
    name: "my-plugin",
    description: "A useful plugin",
    version: "1.2.3",
    tags: ["search", "code"],
    marketplace: "test-market",
  };

  it("renders plugin name, description, version and tags from engine payload", () => {
    server.use(
      http.post("http://127.0.0.1:8001/v1/plugins/install", () => {
        return HttpResponse.json({ ok: true });
      }),
    );
    render(
      <MarketplacePluginCard plugin={ENGINE_PLUGIN} isInstalled={false} />,
      {
        preloadedState: {
          config: {
            apiKey: "test",
            lspPort: 8001,
            themeProps: {},
            host: "vscode",
            addressURL: "Refact",
          },
        },
      },
    );
    expect(screen.getByText("my-plugin")).toBeDefined();
    expect(screen.getByText("A useful plugin")).toBeDefined();
    expect(screen.getByText("1.2.3")).toBeDefined();
    expect(screen.getByText("search")).toBeDefined();
    expect(screen.getByText("code")).toBeDefined();
    expect(screen.getByText("test-market")).toBeDefined();
  });

  it("shows Installed and Uninstall button when isInstalled", () => {
    server.use(
      http.delete(
        "http://127.0.0.1:8001/v1/plugins/installed/my-plugin",
        () => {
          return HttpResponse.json({ deleted: true });
        },
      ),
    );
    render(
      <MarketplacePluginCard plugin={ENGINE_PLUGIN} isInstalled={true} />,
      {
        preloadedState: {
          config: {
            apiKey: "test",
            lspPort: 8001,
            themeProps: {},
            host: "vscode",
            addressURL: "Refact",
          },
        },
      },
    );
    expect(screen.getByText("Installed ✓")).toBeDefined();
    expect(screen.getByText("Uninstall")).toBeDefined();
  });
});

describe("SkillEditor", () => {
  it("renders form fields reflecting loaded skill data", async () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/ext/skills/my_skill", () => {
        return HttpResponse.json({
          name: "my_skill",
          description: "A test skill",
          user_invocable: true,
          disable_model_invocation: false,
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
          addressURL: "Refact",
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
    addressURL: "Refact",
  },
};

describe("Extensions", () => {
  it("shows error state when registry fails to load", async () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/ext/registry", () => {
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
      http.get("http://127.0.0.1:8001/v1/ext/registry", () => {
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

    const confirmTitle = await screen.findByText("Confirm Delete");
    expect(confirmTitle).toBeDefined();
    const cancelBtn = screen.getByText("Cancel");
    expect(cancelBtn).toBeDefined();

    fireEvent.click(cancelBtn);

    await waitFor(() => {
      expect(screen.queryByText("Confirm Delete")).toBeNull();
    });
  });

  it("opens dedicated skills marketplace from skills tab", async () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/ext/registry", () => {
        return HttpResponse.json({
          skills: [],
          slash_commands: [],
          hooks: [],
          has_project_root: true,
        });
      }),
    );

    const { store } = render(
      <Extensions
        host="vscode"
        tabbed={false}
        backFromExtensions={() => undefined}
      />,
      { preloadedState: CONFIG_STATE },
    );

    const button = await screen.findByText("Browse Skills Marketplace");
    fireEvent.click(button);

    const pages = store.getState().pages;
    expect(pages[pages.length - 1]).toEqual({ name: "skills marketplace" });
  });

  it("opens dedicated commands marketplace from commands tab", async () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/ext/registry", () => {
        return HttpResponse.json({
          skills: [],
          slash_commands: [],
          hooks: [],
          has_project_root: true,
        });
      }),
    );

    const { store } = render(
      <Extensions
        host="vscode"
        tabbed={false}
        backFromExtensions={() => undefined}
        initialTab="commands"
      />,
      { preloadedState: CONFIG_STATE },
    );

    const button = await screen.findByText("Browse Commands Marketplace");
    fireEvent.click(button);

    const pages = store.getState().pages;
    expect(pages[pages.length - 1]).toEqual({ name: "commands marketplace" });
  });
});
