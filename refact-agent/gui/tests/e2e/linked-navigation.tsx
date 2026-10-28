import "@radix-ui/themes/styles.css";
import "../../src/styles/tokens.css";
import "../../src/styles/base.css";
import "../../src/styles/glass.css";
import "../../src/styles/motion.css";
import "../../src/styles/responsive.css";
import "../../src/styles/scrollbar.css";
import "../../src/components/Theme/theme-config.css";
import "../../src/components/shared/tokens.css";
import "../../src/lib/render/web.css";
import React from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { Flex } from "@radix-ui/themes";

import { setUpStore, type RootState } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import { TaskWorkspace } from "../../src/features/Tasks/TaskWorkspace";
import { useAllChatsSubscription } from "../../src/hooks/useAllChatsSubscription";
import { setBackendStatus } from "../../src/features/Connection";

const taskId = new URLSearchParams(window.location.search).get("task") ?? "";
const preloadedState: Partial<RootState> = {
  config: {
    host: "web",
    lspPort: 8001,
    lspUrl: "",
    dev: true,
    engineServed: false,
    apiKey: null,
    features: { statistics: true, vecdb: true, ast: true, images: true },
    themeProps: { appearance: "dark" },
    shiftEnterToSubmit: false,
  },
  pages: [{ name: "history" }, { name: "task workspace", taskId }],
};
const store = setUpStore(preloadedState);
store.dispatch(setBackendStatus({ status: "online" }));

const Subscriptions = () => {
  useAllChatsSubscription();
  return null;
};

const root = document.getElementById("refact-chat");
if (!root) throw new Error("Missing #refact-chat root");
createRoot(root).render(
  <Provider store={store}>
    <Theme>
      <AbortControllerProvider>
        <Subscriptions />
        <Flex direction="column" style={{ height: "100vh", minWidth: 0 }}>
          <TaskWorkspace taskId={taskId} />
        </Flex>
      </AbortControllerProvider>
    </Theme>
  </Provider>,
);
