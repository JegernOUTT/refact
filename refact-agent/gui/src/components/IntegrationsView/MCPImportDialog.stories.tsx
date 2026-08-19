import type { Meta, StoryObj } from "@storybook/react";
import { within, userEvent, fireEvent } from "@storybook/testing-library";
import { http, HttpResponse } from "msw";
import type { ProjectConfigResponse } from "../../services/refact/mcpMarketplace";
import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import { MCPImportDialog } from "./MCPImportDialog/MCPImportDialog";

const projectConfig: ProjectConfigResponse = { project_configs: [] };
const sampleJson = JSON.stringify(
  {
    mcpServers: {
      github: {
        command: "npx",
        args: ["-y", "@modelcontextprotocol/server-github"],
        env: { GITHUB_TOKEN: "replace-me" },
      },
    },
  },
  null,
  2,
);

const meta = {
  title: "Integrations/MCPImportDialog",
  component: MCPImportDialog,
  decorators: [
    (Story) => (
      <ChatStoryHarness>
        <Story />
      </ChatStoryHarness>
    ),
  ],
  parameters: {
    msw: {
      handlers: [
        http.get("*/v1/mcp/project-config", () =>
          HttpResponse.json(projectConfig),
        ),
      ],
    },
  },
} satisfies Meta<typeof MCPImportDialog>;

export default meta;
type Story = StoryObj<typeof meta>;

export const OpenWithPastedJson: Story = {
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByRole("button", { name: "Import" }));
    const page = within(canvasElement.ownerDocument.body);
    const dialog = within(await page.findByRole("dialog"));
    const textarea = dialog.getByRole("textbox", { name: "MCP servers JSON" });
    fireEvent.change(textarea, { target: { value: sampleJson } });
  },
};
