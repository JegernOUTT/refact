import { render, screen } from "../utils/test-utils";
import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { server } from "../utils/mockServer";
import { SkillEditor } from "../features/Extensions/components/SkillEditor";
import type { BuddyDraft } from "../features/Buddy/types";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

const COMMAND_DRAFT: BuddyDraft = {
  id: "draft-cmd-1",
  kind: "command",
  title: "My Command Draft",
  yaml_or_json: "---\ndescription: Command desc\n---\n# Command body",
  explanation: "Buddy suggests this command",
  created_at: "2024-01-01T00:00:00Z",
  expires_at: "2099-12-31T00:00:00Z",
};

const MOCK_SKILL_DETAIL = {
  name: "my_skill",
  description: "Existing description",
  user_invocable: true,
  disable_model_invocation: false,
  allowed_tools: [],
  model: null,
  context: null,
  agent: null,
  argument_hint: "",
  body: "# Body",
  raw_content: "---\ndescription: Existing description\n---\n# Body",
  source: "global",
  file_path: "/home/.config/refact/skills/my_skill/SKILL.md",
};

describe("SkillEditor_kind_mismatch_shows_error", () => {
  it("shows kind mismatch when command draft given to skill editor", async () => {
    server.use(
      http.get("http://127.0.0.1:8001/v1/ext/skills/my_skill", () =>
        HttpResponse.json(MOCK_SKILL_DETAIL),
      ),
      http.get("http://127.0.0.1:8001/v1/buddy/drafts/draft-cmd-1", () =>
        HttpResponse.json(COMMAND_DRAFT),
      ),
    );

    render(
      <SkillEditor name="my_skill" onBack={vi.fn()} draftId="draft-cmd-1" />,
      { preloadedState: CONFIG_STATE },
    );

    const mismatch = await screen.findByText(
      "Draft kind mismatch: expected skill draft",
    );
    expect(mismatch).toBeDefined();
  });
});
