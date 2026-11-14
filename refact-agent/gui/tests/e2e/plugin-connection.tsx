import React from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { AbortControllerProvider } from "../../src/contexts/AbortControllers";
import { InnerApp } from "../../src/features/App";
import type { RefactConnectionIssue } from "../../src/features/Config/configSlice";
import "../../src/lib/render/web.css";

const params = new URLSearchParams(location.search);
const requestedIssue = params.get("issue");
const transient =
  requestedIssue === "connecting" || requestedIssue === "installing";
const issue: RefactConnectionIssue | null = transient
  ? null
  : requestedIssue === "workspace_required"
    ? { code: "workspace_required" }
    : requestedIssue === "workspace_busy"
      ? { code: "workspace_busy", holderPid: 4242, holderHttpPort: 8005 }
      : { code: "engine_start_failed", stage: "project_open" };
const store = setUpStore({
  config: {
    host: params.get("host") === "jetbrains" ? "jetbrains" : "vscode",
    apiKey: "fixture-provider-secret-not-for-daemon",
    backendReady: false,
    connectionStatus: transient ? requestedIssue : "failed",
    connectionIssue: issue,
    lspPort: 0,
    daemonPort: 19488,
    runtimeVersions: {
      plugin: {
        state: "observed",
        version: "8.7.0-preview.2",
        source: "plugin",
      },
      requestedVersion: "8.7.0-preview.2",
      selected: { state: "observed", version: "8.6.4", source: "local" },
      installed: null,
      staged: null,
      daemon: { state: "last_observed", version: "8.6.3", source: "daemon" },
      worker: null,
    },
    themeProps: {
      appearance: params.get("theme") === "light" ? "light" : "dark",
    },
  },
  pages: [{ name: "login page" }],
});

createRoot(document.getElementById("refact-chat")!).render(
  <Provider store={store}>
    <AbortControllerProvider>
      <Theme>
        <InnerApp />
      </Theme>
    </AbortControllerProvider>
  </Provider>,
);
