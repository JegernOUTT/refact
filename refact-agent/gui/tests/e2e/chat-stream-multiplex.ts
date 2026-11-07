import { createChatEventMultiplexer } from "../../src/services/refact/chatSubscription";
import {
  sendChatCommand,
  submitQuestionResponse,
} from "../../src/services/refact/chatCommands";

const chatIds = Array.from(
  { length: 8 },
  (_, index) => `visible-chat-${index}`,
);
const ready = new Set<string>();
const errors: string[] = [];
const controllers: AbortController[] = [];
const subscriptions: (() => void)[] = [];
let commandState = "idle";

const harness = {
  async start(engineUrl: string) {
    const config = { host: "web" as const, lspUrl: engineUrl };
    for (let index = 0; index < 3; index += 1) {
      const controller = new AbortController();
      controllers.push(controller);
      const response = await fetch(`${engineUrl}/panel/${index}`, {
        signal: controller.signal,
      });
      if (!response.ok)
        throw new Error(`Panel stream failed: ${response.status}`);
    }
    const multiplexer = createChatEventMultiplexer(config);
    for (const chatId of chatIds) {
      subscriptions.push(
        multiplexer.subscribe(chatId, {
          onEvent: (event) => ready.add(event.chat_id),
          onError: (error) => errors.push(error.message),
        }),
      );
    }
  },
  send(engineUrl: string) {
    commandState = "pending";
    const config = { host: "web" as const, lspUrl: engineUrl };
    void Promise.all([
      sendChatCommand(
        chatIds[0],
        config,
        undefined,
        { type: "user_message", content: "socket regression" },
        undefined,
        "message-request",
      ),
      submitQuestionResponse(
        chatIds[1],
        "pending-question",
        123,
        {
          answers: [{ id: "proceed", disposition: "answered", answer: true }],
          overall_comments: "",
        },
        "question-request",
        config,
      ),
    ]).then(
      () => {
        commandState = "completed";
      },
      (error: unknown) => {
        commandState = "failed";
        errors.push(String(error));
      },
    );
  },
  state() {
    return { ready: [...ready].sort(), commandState, errors: [...errors] };
  },
  stop() {
    subscriptions.splice(0).forEach((unsubscribe) => unsubscribe());
    controllers.splice(0).forEach((controller) => controller.abort());
  },
};

declare global {
  interface Window {
    chatStreamHarness: typeof harness;
  }
}

window.chatStreamHarness = harness;
