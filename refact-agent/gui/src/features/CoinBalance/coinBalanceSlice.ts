import { createSlice } from "@reduxjs/toolkit";
import { smallCloudApi } from "../../services/smallcloud";
import { applyChatEvent } from "../Chat/Thread/actions";

type CoinBalance = {
  balance: number;
};
const initialState: CoinBalance = {
  balance: 0,
};

function extractMeteringBalance(event: unknown): number | null {
  if (typeof event !== "object" || event === null) return null;

  const e = event as Record<string, unknown>;

  if ("metering_balance" in e && typeof e.metering_balance === "number") {
    return e.metering_balance;
  }

  if (e.type === "stream_delta" && Array.isArray(e.ops)) {
    for (const op of e.ops) {
      if (
        typeof op === "object" &&
        op !== null &&
        (op as Record<string, unknown>).op === "merge_extra"
      ) {
        const extra = (op as Record<string, unknown>).extra;
        if (
          typeof extra === "object" &&
          extra !== null &&
          "metering_balance" in extra &&
          typeof (extra as Record<string, unknown>).metering_balance ===
            "number"
        ) {
          return (extra as Record<string, unknown>).metering_balance as number;
        }
      }
    }
  }

  if (
    e.type === "stream_finished" &&
    typeof e.usage === "object" &&
    e.usage !== null
  ) {
    const usage = e.usage as Record<string, unknown>;
    if (
      "metering_balance" in usage &&
      typeof usage.metering_balance === "number"
    ) {
      return usage.metering_balance;
    }
  }

  return null;
}

export const coinBallanceSlice = createSlice({
  name: "coins",
  initialState,
  reducers: {},
  extraReducers: (builder) => {
    builder.addCase(applyChatEvent, (state, action) => {
      const balance = extractMeteringBalance(action.payload);
      if (balance !== null) {
        state.balance = balance;
      }
    });

    builder.addMatcher(
      smallCloudApi.endpoints.getUser.matchFulfilled,
      (state, action) => {
        state.balance = action.payload.metering_balance;
      },
    );
  },

  selectors: {
    selectBalance: (state) => state.balance,
  },
});

export const { selectBalance } = coinBallanceSlice.selectors;
