import { describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "../utils/test-utils";
import { http, HttpResponse } from "msw";
import { server } from "../utils/mockServer";
import { SkillsMarketplace } from "../features/SkillsMarketplace";
import { CommandsMarketplace } from "../features/CommandsMarketplace";
import { SubagentsMarketplace } from "../features/SubagentsMarketplace";

const PRELOADED_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

const REGISTRY = {
  skills: [
    {
      name: "existing-skill",
      description: "Existing",
      source: "global_refact",
      source_label: "Global",
      scope: "local",
      read_only: false,
      file_path: "/tmp/skill",
    },
  ],
  slash_commands: [],
  hooks: [],
  has_project_root: true,
};

const SOURCES = [
  {
    id: "refact-starter-skills",
    label: "Refact Starter Skills",
    description: "Bundled starter skills",
    enabled: true,
    builtin: true,
    removable: false,
    source_kind: "builtin_embedded",
    supported_kinds: ["skill"],
    parser_mode: "scan",
    item_count: 1,
  },
];

describe("SkillsMarketplace", () => {
  it("renders marketplace items from API", async () => {
    server.use(
      http.get("*/v1/ext/registry", () => HttpResponse.json(REGISTRY)),
      http.get("*/v1/skills/marketplace", () =>
        HttpResponse.json({
          items: [
            {
              id: "code-review",
              name: "code-review",
              description: "Review code",
              tags: ["review"],
              publisher: "Refact",
              kind: "skill",
              source_id: "refact-starter-skills",
              source_label: "Refact Starter Skills",
              path: "skills/code-review",
              installed_scopes: [],
            },
          ],
          sources: SOURCES,
        }),
      ),
    );

    render(
      <SkillsMarketplace
        host="vscode"
        tabbed={false}
        backFromMarketplace={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    expect(await screen.findByText("Skills Marketplace")).toBeDefined();
    expect(await screen.findByText("code-review")).toBeDefined();
    expect(screen.getByText("Review code")).toBeDefined();
  });

  it("opens source settings dialog", async () => {
    server.use(
      http.get("*/v1/ext/registry", () => HttpResponse.json(REGISTRY)),
      http.get("*/v1/skills/marketplace", () =>
        HttpResponse.json({ items: [], sources: SOURCES }),
      ),
    );

    render(
      <SkillsMarketplace
        host="vscode"
        tabbed={false}
        backFromMarketplace={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    expect(await screen.findByText("Skills Marketplace")).toBeDefined();
    fireEvent.click(screen.getByTitle("Manage marketplace sources"));
    expect(await screen.findByText("Marketplace Sources")).toBeDefined();
  });
});

describe("CommandsMarketplace", () => {
  it("renders commands from API", async () => {
    server.use(
      http.get("*/v1/ext/registry", () => HttpResponse.json(REGISTRY)),
      http.get("*/v1/commands/marketplace", () =>
        HttpResponse.json({
          items: [
            {
              id: "review",
              name: "review",
              description: "Run review",
              tags: ["review"],
              publisher: "Refact",
              kind: "command",
              source_id: "refact-starter-commands",
              source_label: "Refact Starter Commands",
              path: "commands/review.md",
              installed_scopes: ["global"],
            },
          ],
          sources: [
            {
              id: "refact-starter-commands",
              label: "Refact Starter Commands",
              description: "Bundled starter commands",
              enabled: true,
              builtin: true,
              removable: false,
              source_kind: "builtin_embedded",
              supported_kinds: ["command"],
              parser_mode: "scan",
              item_count: 1,
            },
          ],
        }),
      ),
    );

    render(
      <CommandsMarketplace
        host="vscode"
        tabbed={false}
        backFromMarketplace={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    expect(await screen.findByText("Commands Marketplace")).toBeDefined();
    expect(await screen.findByText("Run review")).toBeDefined();
    expect(screen.getByText("Installed: global")).toBeDefined();
  });
});

describe("SubagentsMarketplace", () => {
  it("renders subagents from API", async () => {
    server.use(
      http.get("*/v1/customization/registry", () =>
        HttpResponse.json({
          modes: [],
          subagents: [],
          toolbox_commands: [],
          code_lens: [],
          errors: [],
          has_project_root: true,
        }),
      ),
      http.get("*/v1/subagents/marketplace", () =>
        HttpResponse.json({
          items: [
            {
              id: "research_helper",
              name: "Research Helper",
              description: "Focused research subagent",
              tags: ["research"],
              publisher: "Refact",
              kind: "subagent",
              source_id: "refact-starter-subagents",
              source_label: "Refact Starter Subagents",
              path: "subagents/research_helper.yaml",
              installed_scopes: [],
            },
          ],
          sources: [
            {
              id: "refact-starter-subagents",
              label: "Refact Starter Subagents",
              description: "Bundled starter subagents",
              enabled: true,
              builtin: true,
              removable: false,
              source_kind: "builtin_embedded",
              supported_kinds: ["subagent"],
              parser_mode: "scan",
              item_count: 1,
            },
          ],
        }),
      ),
    );

    render(
      <SubagentsMarketplace
        host="vscode"
        tabbed={false}
        backFromMarketplace={() => undefined}
      />,
      { preloadedState: PRELOADED_STATE },
    );

    expect(await screen.findByText("Subagents Marketplace")).toBeDefined();
    expect(await screen.findByText("Research Helper")).toBeDefined();
    expect(screen.getByText("Focused research subagent")).toBeDefined();
  });
});
