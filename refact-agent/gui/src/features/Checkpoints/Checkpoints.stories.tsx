import React, { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react";
import { setUpStore } from "../../app/store";
import { Provider } from "react-redux";
import { Theme } from "../../components/Theme";
import { ChatThreadProvider } from "../Chat/Thread";
import { Checkpoints } from "./Checkpoints";
import { CheckpointsMeta } from "./checkpointsSlice";
import {
  makeChatSlice,
  makeChatThread,
} from "../../__stories__/chatStoryState";
import {
  STUB_PREVIEWED_CHECKPOINTS_STATE,
  STUB_RESTORED_CHECKPOINTS_STATE_WITH_NO_CHANGES,
} from "../../__fixtures__/checkpoints";

const Template: React.FC<{ initialState?: CheckpointsMeta }> = ({
  initialState,
}) => {
  const checkpoints = initialState ?? STUB_PREVIEWED_CHECKPOINTS_STATE;
  // useCheckpoints() only shows the popup when the checkpoint result belongs to
  // the *current* thread, so the preloaded chat slice must use the same id as
  // the fixture (`latestCheckpointResult.chat_id`).
  const chatId = checkpoints.latestCheckpointResult.chat_id;
  const [store] = useState(() =>
    setUpStore({
      config: {
        apiKey: "foo",
        host: "web",
        lspPort: 8001,
        themeProps: {
          appearance: "dark",
        },
      },
      chat: makeChatSlice(makeChatThread({ id: chatId })),
      checkpoints,
    }),
  );

  const changedFiles =
    checkpoints.latestCheckpointResult.reverted_changes.flatMap(
      (change) => change.files_changed,
    );

  return (
    <Provider store={store}>
      <Theme>
        <ChatThreadProvider chatId={chatId}>
          {/* The checkpoints dialog is portaled to document.body, so this
              caption keeps the story canvas itself descriptive. */}
          <div style={{ maxWidth: 420, padding: 16 }}>
            <h3>Checkpoints restore dialog</h3>
            <p>
              Dialog is {checkpoints.isVisible ? "open" : "closed"} for chat{" "}
              <code>{chatId}</code>. Changed files: {changedFiles.length}.
            </p>
          </div>
          <Checkpoints />
        </ChatThreadProvider>
      </Theme>
    </Provider>
  );
};

const meta = {
  title: "Features/Checkpoints",
  component: Template,
  parameters: {
    layout: "centered",
  },
} satisfies Meta<typeof Template>;

export default meta;
type Story = StoryObj<typeof Template>;

export const Default: Story = {};

export const WithNoChanges: Story = {
  args: {
    initialState: STUB_RESTORED_CHECKPOINTS_STATE_WITH_NO_CHANGES,
  },
};

export const DialogClosed: Story = {
  args: {
    initialState: {
      ...STUB_RESTORED_CHECKPOINTS_STATE_WITH_NO_CHANGES,
      isVisible: false,
    },
  },
};
