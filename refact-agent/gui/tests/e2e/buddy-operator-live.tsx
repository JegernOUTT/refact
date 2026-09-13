import React from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { BuddyDesk } from "../../src/features/Buddy/BuddyDesk";
import { BuddyNativeDecisions } from "../../src/features/Buddy/BuddyNativeDecisions";
import "../../src/lib/render/web.css";

// Composition only: all queries, mutations, decisions and execution UI are production.
// Authentication is the launcher's public scratch token, never a user credential.
const store = setUpStore({
  config: {
    host: "web",
    dev: true,
    apiKey: "scratch-only-not-a-secret",
    themeProps: { appearance: "dark" },
  },
});
const root = document.getElementById("refact-chat");
if (!root) throw new Error("Missing confined acceptance root");
createRoot(root).render(
  <Provider store={store}>
    <Theme>
      <BuddyDesk />
      <BuddyNativeDecisions />
    </Theme>
  </Provider>,
);
