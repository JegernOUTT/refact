import { buddyApi } from "./buddy";
import { buildApiUrlFromState, type EngineApiConfig } from "./apiUrl";
import { buddyOperatorIdentity } from "./buddyOperator";

export type EditorialFeedback = "useful" | "noisy" | "less_like_this";
export type EditorialIdentity = {
  endpoint_identity: string;
  project: import("../../features/Buddy/operatorTypes").ProjectIdentity;
  worker: import("../../features/Buddy/operatorTypes").WorkerIdentity;
};
type State = Parameters<typeof buddyOperatorIdentity>[0];
export function assertEditorialIdentity(
  state: unknown,
  identity: string,
): void {
  if (buddyOperatorIdentity(state as State) !== identity)
    throw new Error("Project connection changed");
}
export function parseEditorialAck(
  value: unknown,
  field: "claimed" | "recorded",
): boolean {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("Invalid editorial acknowledgement");
  const v = value as Record<string, unknown>;
  if (
    Object.keys(v).length !== 2 ||
    typeof v[field] !== "boolean" ||
    v.permission_granted !== false
  )
    throw new Error("Editorial feedback cannot grant permission");
  return v[field];
}
export function validateSpeechId(id: string): void {
  if (!/^[a-zA-Z0-9_:-]{1,128}$/.test(id))
    throw new Error("Invalid speech identity");
}
export const buddyEditorialApi = buddyApi.injectEndpoints({
  endpoints: (builder) => ({
    claimBuddyEditorial: builder.mutation<
      boolean,
      EditorialIdentity & {
        speech_id: string;
        surface: "panel" | "notification";
      }
    >({
      queryFn: async (args, api, _options, baseQuery) => {
        try {
          assertEditorialIdentity(api.getState(), args.endpoint_identity);
          validateSpeechId(args.speech_id);
          if (!["panel", "notification"].includes(args.surface))
            throw new Error("Avatar delivery is not activated");
          const result = await baseQuery({
            url: buildApiUrlFromState(
              api.getState() as { config: EngineApiConfig },
              "/v1/buddy/editorial/claim",
            ),
            method: "POST",
            body: {
              project: args.project,
              worker: args.worker,
              speech_id: args.speech_id,
              surface: args.surface,
            },
          });
          assertEditorialIdentity(api.getState(), args.endpoint_identity);
          if (result.error) return { error: result.error };
          return { data: parseEditorialAck(result.data, "claimed") };
        } catch {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: "Editorial delivery unconfirmed; speech withheld",
            },
          };
        }
      },
    }),
    feedbackBuddyEditorial: builder.mutation<
      boolean,
      EditorialIdentity & { speech_id: string; feedback: EditorialFeedback }
    >({
      queryFn: async (args, api, _options, baseQuery) => {
        try {
          assertEditorialIdentity(api.getState(), args.endpoint_identity);
          validateSpeechId(args.speech_id);
          if (!["useful", "noisy", "less_like_this"].includes(args.feedback))
            throw new Error("Invalid feedback");
          const result = await baseQuery({
            url: buildApiUrlFromState(
              api.getState() as { config: EngineApiConfig },
              "/v1/buddy/editorial/feedback",
            ),
            method: "POST",
            body: {
              project: args.project,
              worker: args.worker,
              speech_id: args.speech_id,
              feedback: args.feedback,
            },
          });
          assertEditorialIdentity(api.getState(), args.endpoint_identity);
          if (result.error) return { error: result.error };
          return { data: parseEditorialAck(result.data, "recorded") };
        } catch {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: "Editorial feedback unconfirmed",
            },
          };
        }
      },
    }),
  }),
});
export const {
  useClaimBuddyEditorialMutation,
  useFeedbackBuddyEditorialMutation,
} = buddyEditorialApi;
