import { expect, test } from "vitest";
import { isHostMessage } from "../isHostMessage";

test("host events remain supported while embedded frame events are rejected", () => {
  expect(isHostMessage(new MessageEvent("message"))).toBe(true);
  expect(isHostMessage(new MessageEvent("message", { source: window }))).toBe(
    true,
  );
  const frame = document.createElement("iframe");
  document.body.append(frame);
  expect(
    isHostMessage(new MessageEvent("message", { source: frame.contentWindow })),
  ).toBe(false);
  frame.remove();
});
