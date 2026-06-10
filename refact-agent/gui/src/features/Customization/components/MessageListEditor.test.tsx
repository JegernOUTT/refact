import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { MessageListEditor } from "./MessageListEditor";

function ControlledMessageList() {
  const [messages, setMessages] = useState([
    { role: "user", content: "hello" },
  ]);

  return <MessageListEditor value={messages} onChange={setMessages} />;
}


describe("MessageListEditor", () => {
  it("keeps the focused textarea mounted across controlled edits", async () => {
    const { user } = render(<ControlledMessageList />);
    const textarea = screen.getByPlaceholderText("Message content...");

    await user.click(textarea);
    await user.type(textarea, "!");

    expect(document.activeElement).toBe(textarea);
    expect(textarea).toHaveValue("hello!");
  });

  it("emits message updates without internal ids", async () => {
    const onChange = vi.fn();
    const { user } = render(
      <MessageListEditor
        value={[{ role: "user", content: "hello" }]}
        onChange={onChange}
      />,
    );

    await user.type(screen.getByPlaceholderText("Message content..."), "!");

    expect(onChange).toHaveBeenLastCalledWith([
      { role: "user", content: "hello!" },
    ]);
  });
});
