import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "../../utils/test-utils";
import { setUpStore } from "../../app/store";
import { updateConfig } from "../../features/Config/configSlice";
import { Link } from "./Link";

describe("content anchor target", () => {
  it("leaves a primary target blank click native without invoking in-place navigation", () => {
    const store = setUpStore();
    store.dispatch(updateConfig({ host: "web" }));
    const navigate = vi.fn((event: React.MouseEvent) => event.preventDefault());
    render(
      <Link href="refact://chat/linked" target="_blank" onClick={navigate}>
        Chat
      </Link>,
      { store },
    );
    const link = screen.getByRole("link", { name: "Chat" });
    expect(link).toHaveAttribute("href", "/#/chat/linked");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
    expect(fireEvent.click(link)).toBe(true);
    expect(navigate).not.toHaveBeenCalled();
  });
});
