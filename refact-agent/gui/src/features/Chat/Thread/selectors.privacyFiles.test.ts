import { describe, expect, it } from "vitest";
import type { RootState } from "../../../app/store";
import type { PrivacyFileRecord } from "../../../services/refact/privacy";
import type { ChatMessages } from "../../../services/refact/types";
import { selectPrivacyFilesById } from "./selectors";

const envFile: PrivacyFileRecord = {
  path: ".env.production",
  zone: "secrets",
  attribution: "observed",
};

const keyFile: PrivacyFileRecord = {
  path: "config/key.pem",
  zone: "secrets",
  attribution: "declared",
};

const envFileOtherZone: PrivacyFileRecord = {
  path: ".env.production",
  zone: "workspace",
  attribution: "observed",
};

const envFileOtherAttribution: PrivacyFileRecord = {
  path: ".env.production",
  zone: "secrets",
  attribution: "heuristic",
};

function makeToolMessage(
  toolCallId: string,
  files: PrivacyFileRecord[],
): ChatMessages[number] {
  return {
    role: "tool",
    tool_call_id: toolCallId,
    content: `result ${toolCallId}`,
    extra: { privacy: { files } },
  } as ChatMessages[number];
}

function makeState(messages: ChatMessages): RootState {
  return {
    chat: {
      current_thread_id: "thread-A",
      open_thread_ids: ["thread-A"],
      threads: {
        "thread-A": {
          thread: {
            id: "thread-A",
            messages,
          },
        },
      },
    },
  } as unknown as RootState;
}

describe("selectPrivacyFilesById", () => {
  it("dedupes records across messages and keeps first-occurrence order", () => {
    const duplicateEnvFile = { ...envFile };
    const state = makeState([
      makeToolMessage("tool-A", [envFile, keyFile]),
      makeToolMessage("tool-B", [duplicateEnvFile, keyFile]),
      makeToolMessage("tool-C", [envFile]),
    ]);

    const files = selectPrivacyFilesById(state, "thread-A");

    expect(files).toEqual([envFile, keyFile]);
    expect(files[0]).toBe(envFile);
    expect(files[1]).toBe(keyFile);
  });

  it("keeps records that differ only by zone or attribution", () => {
    const state = makeState([
      makeToolMessage("tool-A", [envFile, envFileOtherZone]),
      makeToolMessage("tool-B", [envFileOtherAttribution, envFile]),
    ]);

    expect(selectPrivacyFilesById(state, "thread-A")).toEqual([
      envFile,
      envFileOtherZone,
      envFileOtherAttribution,
    ]);
  });

  it("ignores messages without privacy metadata and returns an empty list", () => {
    const state = makeState([
      { role: "user", content: "hello" } as ChatMessages[number],
      makeToolMessage("tool-A", []),
    ]);

    expect(selectPrivacyFilesById(state, "thread-A")).toEqual([]);
  });

  it("collapses a large duplicate-heavy transcript to its unique records", () => {
    const messages = Array.from({ length: 40 }, (_, index) =>
      makeToolMessage(
        `tool-${index}`,
        Array.from({ length: 400 }, () => ({ ...envFile })),
      ),
    );

    expect(selectPrivacyFilesById(makeState(messages), "thread-A")).toEqual([
      envFile,
    ]);
  });
});
