import { StrictMode } from "react";
import { type Config, updateConfig } from "../../features/Config/configSlice";
import { App } from "../../features/App";
import { reportBuddyFrontendError } from "../../features/Buddy/reportBuddyFrontendError";
import { withBuddyErrorReport } from "../../features/Buddy/BuddyErrorBoundary";
import ReactDOM from "react-dom/client";
import { store } from "../../app/store";
import "./web.css";

export function renderApp(element: HTMLElement, config?: Partial<Config>) {
  if (config) {
    store.dispatch(updateConfig(config));
  }

  const root = withBuddyErrorReport(
    () =>
      ReactDOM.createRoot(element, {
        onRecoverableError(error) {
          void reportBuddyFrontendError({
            source: "react_recoverable",
            error,
            sourceFile: "frontend/react_recoverable",
            toolName: "react_recoverable",
          });
        },
      }),
    {
      source: "react_root_render",
      sourceFile: "frontend/react_root_create",
      toolName: "react_root_create",
    },
  );

  withBuddyErrorReport(
    () =>
      root.render(
        <StrictMode>
          <App />
        </StrictMode>,
      ),
    {
      source: "react_root_render",
      sourceFile: "frontend/react_root_render",
      toolName: "react_root_render",
    },
  );
}
