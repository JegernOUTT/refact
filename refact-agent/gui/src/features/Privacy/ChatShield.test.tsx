import { HttpResponse, http } from "msw";
import { describe, expect, it } from "vitest";

import {
  createDefaultChatState,
  render,
  screen,
  waitFor,
  within,
} from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import type { ChatMessages } from "../../services/refact/types";
import type {
  PrivacyDestination,
  PrivacyFileRecord,
  PrivacyPolicyResponse,
} from "../../services/refact/privacy";
import { ChatShield } from "./ChatShield";

const destination: PrivacyDestination = {
  id: "untrusted",
  kind: "provider",
  display_name: "untrusted/model",
};

const policy: PrivacyPolicyResponse = {
  policy: {
    blocked: [],
    zones: [],
    subagents: { report_declassifies: true },
    tool_access: { providers: {} },
  },
  destinations: [destination],
  match_counts: {},
  error: null,
  source_paths: [],
  has_project_overrides: false,
};

const secretFile: PrivacyFileRecord = {
  path: ".env",
  zone: "secrets",
  attribution: "observed",
};
const internalFile: PrivacyFileRecord = {
  path: "docs/plan.md",
  zone: "internal",
  attribution: "declared",
};
const blockedFile: PrivacyFileRecord = {
  path: "id_rsa",
  zone: "blocked",
  attribution: "observed",
};
const normalFile: PrivacyFileRecord = {
  path: "README.md",
  zone: "normal",
  attribution: "observed",
};

type ClearBody = { chat_id: string; paths: string[] };

function setup(
  initialRecords: PrivacyFileRecord[],
  clearStatus = 200,
  persisted = true,
): { clears: ClearBody[]; inspects: () => number } {
  let records = initialRecords;
  let inspectCount = 0;
  const clears: ClearBody[] = [];
  server.use(
    http.get("*/v1/privacy/policy", () => HttpResponse.json(policy)),
    http.post("*/v1/privacy/inspect", () => {
      inspectCount += 1;
      return HttpResponse.json({
        chat_id: "chat",
        destination,
        sendable: true,
        records,
        blocked: [],
        refusal: null,
      });
    }),
    http.post("*/v1/privacy/records/clear", async ({ request }) => {
      const body = (await request.json()) as ClearBody;
      clears.push(body);
      if (clearStatus !== 200) {
        return HttpResponse.json(
          { detail: "operator auth failed" },
          { status: clearStatus },
        );
      }
      const before = records.length;
      records = records.filter((record) => !body.paths.includes(record.path));
      return HttpResponse.json({
        cleared: before - records.length,
        remaining: records.length,
        persisted,
      });
    }),
  );
  return { clears, inspects: () => inspectCount };
}

function renderShield() {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.model = destination.display_name;
  runtime.thread.messages = [] as ChatMessages;
  const view = render(<ChatShield threadId={chat.current_thread_id} />, {
    preloadedState: { chat },
  });
  return { ...view, chatId: chat.current_thread_id };
}

async function openLabels(user: ReturnType<typeof render>["user"]) {
  await user.click(
    await screen.findByRole("button", { name: "Manage privacy labels" }),
  );
  return screen.findByRole("list", { name: "Privacy labels" });
}

async function confirm(user: ReturnType<typeof render>["user"]) {
  const dialog = await screen.findByRole("dialog", {
    name: "Clear privacy label",
  });
  await user.click(
    within(dialog).getByRole("button", { name: "Remove label" }),
  );
}

describe("ChatShield privacy labels", () => {
  it("warns non-blockingly when cleared labels have not been persisted", async () => {
    setup([secretFile], 200, false);
    const { user } = renderShield();
    await openLabels(user);
    await user.click(
      screen.getByRole("button", { name: "Clear privacy label for .env" }),
    );
    await confirm(user);
    const dialog = await screen.findByRole("dialog", {
      name: "Privacy labels",
    });
    expect(await within(dialog).findByRole("status")).toHaveTextContent(
      "Labels were removed for this session but could not be saved yet; they will be saved on the next successful save.",
    );
    expect(within(dialog).getByRole("button", { name: "Close" })).toBeEnabled();
  });
  it("clears one record after confirmation and refetches inspection", async () => {
    const { clears, inspects } = setup([secretFile, internalFile]);
    const { user, chatId } = renderShield();

    await openLabels(user);
    await user.click(
      screen.getByRole("button", { name: "Clear privacy label for .env" }),
    );
    expect(
      await screen.findByText(
        'Remove this privacy label from the chat? Data already in this chat may then be sent to destinations outside zone "secrets".',
      ),
    ).toBeInTheDocument();
    const inspectsBefore = inspects();
    await confirm(user);

    await waitFor(() => expect(clears).toHaveLength(1));
    expect(clears[0]).toEqual({ chat_id: chatId, paths: [".env"] });
    await waitFor(() => expect(inspects()).toBeGreaterThan(inspectsBefore));
    await waitFor(() =>
      expect(
        screen.queryByRole("button", {
          name: "Clear privacy label for .env",
        }),
      ).not.toBeInTheDocument(),
    );
    expect(
      screen.getByRole("button", {
        name: "Clear privacy label for docs/plan.md",
      }),
    ).toBeInTheDocument();
  });

  it("clears all non-normal, non-blocked records", async () => {
    const { clears } = setup([
      secretFile,
      internalFile,
      blockedFile,
      normalFile,
    ]);
    const { user, chatId } = renderShield();

    await openLabels(user);
    await user.click(screen.getByRole("button", { name: "Clear all" }));
    expect(
      await screen.findByText(/outside zones "secrets", "internal"\./),
    ).toBeInTheDocument();
    await confirm(user);

    await waitFor(() => expect(clears).toHaveLength(1));
    expect(clears[0]).toEqual({
      chat_id: chatId,
      paths: [".env", "docs/plan.md"],
    });
  });

  it("does not offer to clear blocked or normal records", async () => {
    setup([secretFile, blockedFile, normalFile]);
    const { user } = renderShield();

    const list = await openLabels(user);
    expect(within(list).getByText("id_rsa")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Clear privacy label for id_rsa" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", {
        name: "Clear privacy label for README.md",
      }),
    ).not.toBeInTheDocument();
  });

  it("hides the labels action when nothing can be cleared", async () => {
    setup([blockedFile, normalFile]);
    renderShield();

    await screen.findByText("Everything here can go to untrusted/model");
    expect(
      screen.queryByRole("button", { name: "Manage privacy labels" }),
    ).not.toBeInTheDocument();
  });

  it("does not clear when the confirmation is cancelled", async () => {
    const { clears } = setup([secretFile]);
    const { user } = renderShield();

    await openLabels(user);
    await user.click(
      screen.getByRole("button", { name: "Clear privacy label for .env" }),
    );
    const dialog = await screen.findByRole("dialog", {
      name: "Clear privacy label",
    });
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() =>
      expect(
        screen.queryByRole("dialog", { name: "Clear privacy label" }),
      ).not.toBeInTheDocument(),
    );
    expect(clears).toHaveLength(0);
  });

  it("shows an error when clearing is refused", async () => {
    const { clears } = setup([secretFile], 403);
    const { user } = renderShield();

    await openLabels(user);
    await user.click(
      screen.getByRole("button", { name: "Clear privacy label for .env" }),
    );
    await confirm(user);

    await waitFor(() => expect(clears).toHaveLength(1));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "You are not allowed to clear privacy labels for this chat.",
    );
    expect(
      screen.getByRole("button", { name: "Clear privacy label for .env" }),
    ).toBeInTheDocument();
  });
});
