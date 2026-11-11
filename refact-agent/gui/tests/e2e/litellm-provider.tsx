import React from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { ProviderForm } from "../../src/features/Providers/ProviderForm/ProviderForm";

const store = setUpStore({
  config: { host: "web", dev: true, apiKey: "fixture", themeProps: {} },
});
const root = document.getElementById("refact-chat");
if (!root) throw new Error("Missing fixture root");
createRoot(root).render(
  <Provider store={store}>
    <Theme>
      <ProviderForm
        currentProvider={{
          name: "litellm_team",
          base_provider: "litellm",
          display_name: "Team LiteLLM",
          enabled: true,
          readonly: false,
          has_credentials: true,
          status: "active",
          model_count: 0,
        }}
      />
    </Theme>
  </Provider>,
);
