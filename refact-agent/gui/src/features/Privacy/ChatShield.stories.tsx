import type { Meta, StoryObj } from "@storybook/react";
import { http, HttpResponse } from "msw";

import { ChatStoryHarness } from "../../__stories__/ChatStoryHarness";
import {
  makeChatThread,
  STORY_CHAT_ID,
} from "../../__stories__/chatStoryState";
import type {
  PrivacyDestination,
  PrivacyFileRecord,
  PrivacyInspectResponse,
  PrivacyPolicyResponse,
} from "../../services/refact/privacy";
import type { ChatMessages } from "../../services/refact/types";
import { ChatShield } from "./ChatShield";

const destination: PrivacyDestination = {
  id: "openai",
  kind: "provider",
  display_name: "openai/gpt-4o",
};

const guardedFile: PrivacyFileRecord = {
  path: ".env.production",
  zone: "secrets",
  attribution: "observed",
};

const policy: PrivacyPolicyResponse = {
  policy: {
    blocked: [],
    zones: [
      {
        name: "secrets",
        patterns: [".env*"],
        send_to: ["trusted-local"],
        on_shell_read: "withhold",
      },
    ],
    subagents: { report_declassifies: true },
    tool_access: { providers: {} },
  },
  destinations: [destination],
  match_counts: { secrets: 1 },
  error: null,
  source_paths: [".refact/privacy.yaml"],
  has_project_overrides: true,
};

const inspection: PrivacyInspectResponse = {
  chat_id: STORY_CHAT_ID,
  destination,
  sendable: false,
  would_send: [],
  records: [guardedFile],
  blocked: [{ record_index: 0, record: guardedFile }],
  refusal: "The selected provider cannot receive files from the secrets zone.",
};

const messages = [
  {
    role: "tool",
    message_id: "privacy-file-result",
    tool_call_id: "privacy-file-call",
    content: "Output withheld by user privacy policy.",
    extra: { privacy: { files: [guardedFile] } },
  },
] satisfies ChatMessages;

function ChatShieldStory() {
  const thread = makeChatThread({
    model: destination.display_name,
    messages,
  });
  return (
    <ChatStoryHarness thread={thread} height="240px">
      <ChatShield threadId={STORY_CHAT_ID} />
    </ChatStoryHarness>
  );
}

const meta = {
  title: "Privacy/Chat Shield",
  component: ChatShieldStory,
  parameters: {
    msw: {
      handlers: [
        http.get("*/v1/privacy/policy", () => HttpResponse.json(policy)),
        http.post("*/v1/privacy/inspect", () => HttpResponse.json(inspection)),
      ],
    },
  },
} satisfies Meta<typeof ChatShieldStory>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
