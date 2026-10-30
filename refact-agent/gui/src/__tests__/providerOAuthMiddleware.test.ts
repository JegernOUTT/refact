import { describe, expect, test } from "vitest";
import { http, HttpResponse } from "msw";
import { setUpStore } from "../app/store";
import { STUB_CAPS_RESPONSE } from "../__fixtures__/caps";
import { setError } from "../features/Errors/errorsSlice";
import { capsApi, modelsApi, providersApi } from "../services/refact";
import { server } from "../utils/mockServer";

async function seededStore() {
  server.use(
    http.post("*/v1/buddy/diagnostics/collect", () =>
      HttpResponse.json({ success: true }),
    ),
  );
  const store = setUpStore({
    config: { apiKey: "test", lspPort: 8001, themeProps: {}, host: "vscode" },
  });
  await store.dispatch(
    capsApi.util.upsertQueryData("getCaps", undefined, STUB_CAPS_RESPONSE),
  );
  await store.dispatch(
    modelsApi.util.upsertQueryData(
      "getModels",
      { providerName: "claude_code" },
      {
        chat_models: [],
        completion_models: [],
        embedding_model: null,
      },
    ),
  );
  store.dispatch(setError("Keep this diagnostic"));
  return store;
}

const exchangeArgs = {
  providerName: "claude_code",
  session_id: "native-1",
  code: "",
};

describe("provider OAuth middleware", () => {
  test.each(["pending", "failed"])(
    "preserves caches and errors for HTTP 200 %s",
    async (status) => {
      server.use(
        http.post("*/v1/providers/claude_code/oauth/exchange", () =>
          HttpResponse.json({
            success: false,
            status,
            auth_status: "AWS sign-in diagnostic",
          }),
        ),
      );
      const store = await seededStore();
      const before = store.getState();
      const result = await store.dispatch(
        providersApi.endpoints.oauthExchange.initiate(exchangeArgs),
      );
      expect(result.data?.success).toBe(false);
      expect(store.getState().error).toEqual(before.error);
      expect(store.getState().caps.queries).toEqual(before.caps.queries);
      expect(store.getState().models.queries).toEqual(before.models.queries);
    },
  );

  test.each(["exchange", "save"])(
    "retains cache resets and error clearing for successful %s",
    async (operation) => {
      server.use(
        http.post("*/v1/providers/claude_code/oauth/exchange", () =>
          HttpResponse.json({ success: true, auth_status: "Connected" }),
        ),
        http.post("*/v1/providers/claude_code", () =>
          HttpResponse.json({ success: true }),
        ),
      );
      const store = await seededStore();
      if (operation === "exchange") {
        await store.dispatch(
          providersApi.endpoints.oauthExchange.initiate(exchangeArgs),
        );
      } else {
        await store
          .dispatch(
            providersApi.endpoints.updateProvider.initiate({
              providerName: "claude_code",
              settings: { enabled: true },
            }),
          )
          .unwrap();
      }
      expect(store.getState().error.message).toBeNull();
      expect(store.getState().caps.queries).toEqual({});
      expect(store.getState().models.queries).toEqual({});
    },
  );
});
