import { describe, expect, it } from "vitest";

import { STUB_SHELL_POLICY } from "../../__fixtures__/shellPolicy";
import { isShellPolicy } from "./shellPolicy";

describe("isShellPolicy", () => {
  it("accepts the engine payload without a foreground timeout", () => {
    expect(isShellPolicy(STUB_SHELL_POLICY)).toBe(true);
  });

  it("rejects a missing execution output line limit", () => {
    expect(isShellPolicy({ ...STUB_SHELL_POLICY, execution: {} })).toBe(false);
  });

  it("rejects an invalid approval mode", () => {
    expect(isShellPolicy({ ...STUB_SHELL_POLICY, mode: "invalid" })).toBe(
      false,
    );
  });

  it("rejects a catalogue entry with an invalid risk level", () => {
    expect(
      isShellPolicy({
        ...STUB_SHELL_POLICY,
        catalogue: STUB_SHELL_POLICY.catalogue.map((entry, index) =>
          index === 1 ? { ...entry, level: "invalid" } : entry,
        ),
      }),
    ).toBe(false);
  });
});
