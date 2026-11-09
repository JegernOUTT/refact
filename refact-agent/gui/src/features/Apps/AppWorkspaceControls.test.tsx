import { afterEach, describe, expect, it, vi } from "vitest";
import { setUpStore } from "../../app/store";
import { cleanup, render, screen } from "../../utils/test-utils";
import { AppWorkspaceControls } from "./AppWorkspaceControls";
import { retryAppWorkspaceSave } from "./appsSlice";
import styles from "./AppWorkspaceControls.module.css";

afterEach(cleanup);

type SaveStatus = "saving" | "saved" | "error" | "readonly";

function setup(status?: SaveStatus, surface = "chat:owner") {
  const initial = setUpStore().getState();
  const store = setUpStore({
    chat: {
      ...initial.chat,
      open_thread_ids: [...initial.chat.open_thread_ids, "owner"],
    },
    workspace: {
      ...initial.workspace,
      tabs: [surface],
      activeTabId: surface,
      groups: {},
    },
    apps: {
      views: {
        "app:timer": { appId: "timer", chatId: "app-owner", name: "Timer" },
      },
      stagedByChat: {},
      saveStatus: {
        owner: status,
        "app-owner": status,
        [initial.chat.current_thread_id]: "error",
      },
    },
  });
  const dispatch = vi.spyOn(store, "dispatch");
  return { ...render(<AppWorkspaceControls />, { store }), dispatch };
}

describe("AppWorkspaceControls", () => {
  it("shows a persistent, polite warning with an explicit retry for the focused chat", async () => {
    const view = setup("error");
    const notice = screen.getByRole("status", { name: "App layout" });
    expect(notice).toHaveTextContent(
      "App layout could not be saved. Your local layout is preserved.",
    );
    expect(notice).toHaveAttribute("aria-live", "polite");
    expect(notice).toHaveAttribute("aria-atomic", "true");
    expect(notice).toHaveClass(styles.notice);
    expect(notice.querySelector("svg")).toHaveAttribute("aria-hidden", "true");
    await view.user.click(screen.getByRole("button", { name: "Retry save" }));
    expect(view.dispatch).toHaveBeenCalledWith(retryAppWorkspaceSave("owner"));
    expect(notice).toBeInTheDocument();
  });

  it("shows read-only information without offering a save retry", () => {
    setup("readonly");
    const notice = screen.getByRole("status", { name: "App layout" });
    expect(notice).toHaveTextContent(
      "This app workspace version is read-only.",
    );
    expect(notice).toHaveAttribute("data-status", "readonly");
    expect(notice).toHaveClass(styles.notice);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it.each([
    ["saving", "Saving app layout…"],
    ["saved", "App layout saved automatically."],
  ] as const)(
    "keeps %s feedback quiet rather than a permanent banner",
    (status, message) => {
      setup(status);
      const notice = screen.getByRole("status", { name: "App layout" });
      expect(notice).toHaveTextContent(message);
      expect(notice).toHaveClass(styles.quietStatus);
      expect(notice).not.toHaveClass(styles.notice);
      expect(screen.queryByRole("button")).not.toBeInTheDocument();
    },
  );

  it("retries the focused app's owner rather than the unrelated current thread", async () => {
    const view = setup("error", "app:timer");
    await view.user.click(screen.getByRole("button", { name: "Retry save" }));
    expect(view.dispatch).toHaveBeenCalledWith(
      retryAppWorkspaceSave("app-owner"),
    );
    expect(view.dispatch).not.toHaveBeenCalledWith(
      retryAppWorkspaceSave(view.store.getState().chat.current_thread_id),
    );
  });

  it.each(["settings", "app:unknown"])(
    "does not show another chat's error on unrelated surface %s",
    (surface) => {
      setup("error", surface);
      expect(screen.queryByRole("status")).not.toBeInTheDocument();
    },
  );

  it("renders no notice when the focused chat has no save status", () => {
    setup();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });
});
