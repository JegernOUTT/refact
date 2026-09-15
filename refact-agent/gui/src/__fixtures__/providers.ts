import { http, HttpResponse, delay, type HttpHandler } from "msw";

import { STUB_CAPS_RESPONSE } from "./caps";
import type { ProviderListItem } from "../services/refact";

export const activeProvider: ProviderListItem = {
  name: "openai",
  base_provider: "openai",
  display_name: "OpenAI",
  enabled: true,
  readonly: false,
  has_credentials: true,
  status: "active",
  model_count: 1,
};

export const inactiveProvider: ProviderListItem = {
  ...activeProvider,
  status: "configured",
};

export function capsFailsOnceThenSucceeds({
  successDelayMs,
}: { successDelayMs?: number } = {}): {
  handler: HttpHandler;
  hits: () => number;
} {
  let hits = 0;
  const handler = http.get("*/v1/caps", async () => {
    hits += 1;
    if (hits === 1) {
      return HttpResponse.json({ detail: "caps exploded" }, { status: 500 });
    }
    if (successDelayMs !== undefined) {
      await delay(successDelayMs);
    }
    return HttpResponse.json(STUB_CAPS_RESPONSE);
  });
  return { handler, hits: () => hits };
}
