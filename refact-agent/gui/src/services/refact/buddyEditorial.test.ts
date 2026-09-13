import { describe, expect, it } from "vitest";
import {
  assertEditorialIdentity,
  parseEditorialAck,
  validateSpeechId,
} from "./buddyEditorial";
import { buddyOperatorIdentity } from "./buddyOperator";

describe("editorial transport contract", () => {
  it("accepts only permission-free boolean acknowledgements", () => {
    expect(
      parseEditorialAck(
        { claimed: true, permission_granted: false },
        "claimed",
      ),
    ).toBe(true);
    expect(
      parseEditorialAck(
        { claimed: false, permission_granted: false },
        "claimed",
      ),
    ).toBe(false);
    for (const value of [
      null,
      {},
      { claimed: "true", permission_granted: false },
      { claimed: true, permission_granted: true },
      { claimed: true, permission_granted: false, settings: {} },
    ])
      expect(() => parseEditorialAck(value, "claimed")).toThrow();
    expect(
      parseEditorialAck(
        { recorded: true, permission_granted: false },
        "recorded",
      ),
    ).toBe(true);
  });
  it("rejects unsafe or oversized speech identities", () => {
    expect(() => validateSpeechId("speech:one-2")).not.toThrow();
    for (const id of ["", "a".repeat(129), "private/token", "token=secret"])
      expect(() => validateSpeechId(id)).toThrow();
  });
  it("binds project and endpoint before and after asynchronous work", () => {
    const state = {
      config: { host: "vscode" as const, lspPort: 8001 },
      current_project: { name: "one", workspaceRoots: ["/one"] },
    };
    const identity = buddyOperatorIdentity(state);
    expect(() => assertEditorialIdentity(state, identity)).not.toThrow();
    expect(() =>
      assertEditorialIdentity(
        {
          ...state,
          current_project: { name: "two", workspaceRoots: ["/two"] },
        },
        identity,
      ),
    ).toThrow();
    expect(() =>
      assertEditorialIdentity(
        { ...state, config: { ...state.config, lspPort: 8002 } },
        identity,
      ),
    ).toThrow();
  });
});
