import React from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { Flex } from "@radix-ui/themes";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { setThemeMode } from "../../src/features/Config/configSlice";
import { GroupSplitView } from "../../src/features/Workspace/GroupSplitView";
import { WorkspaceToolbarTabs } from "../../src/features/Workspace/WorkspacePaneTabs";
import "../../src/lib/render/web.css";
import styles from "./workspace-showcase.module.css";

const originalFetch = window.fetch.bind(window);
window.fetch = (input, init) => {
  const url = input instanceof Request ? input.url : String(input);
  if (url.includes("/v1/"))
    return Promise.resolve(
      new Response(JSON.stringify({ detail: "Fixture file unavailable" }), {
        status: 404,
        headers: { "Content-Type": "application/json" },
      }),
    );
  return originalFetch(input, init);
};

const initial = setUpStore().getState();
const store = setUpStore({
  ...initial,
  pages: [{ name: "chat" }],
  workspace: {
    ...initial.workspace,
    tabs: ["file:/left"],
    activeTabId: "file:/left",
    groups: {
      "file:/left": {
        focusedLeafId: "right",
        root: {
          kind: "split",
          id: "split",
          dir: "row",
          sizes: [0.5, 0.5],
          children: [
            {
              kind: "leaf",
              id: "left",
              tabIds: ["file:/left"],
              activeTabId: "file:/left",
            },
            {
              kind: "leaf",
              id: "right",
              tabIds: ["file:/right"],
              activeTabId: "file:/right",
            },
          ],
        },
      },
    },
  },
});
store.dispatch(setThemeMode("dark"));

const root = document.getElementById("refact-chat");
if (!root) throw new Error("Missing workspace showcase root");
createRoot(root).render(
  <Provider store={store}>
    <Theme>
      <Flex direction="column" className={styles.page}>
        <div className={styles.toolbar}>
          <WorkspaceToolbarTabs />
        </div>
        <Flex className={styles.body}>
          <GroupSplitView tabId="file:/left" />
        </Flex>
      </Flex>
    </Theme>
  </Provider>,
);
