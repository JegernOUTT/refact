import { type ReactNode } from "react";
import { useAppSelector } from "../../hooks";
import { useIsDocumentVisible } from "../../hooks/useIsDocumentVisible";
import {
  buddyOperatorIdentity,
  useGetBuddyOperatorWorkQuery,
} from "../../services/refact/buddyOperator";
import { useGetBuddyNativeWorkQuery } from "../../services/refact/buddyNativeWork";
import { useGetBuddyNativeDecisionsQuery } from "../../services/refact/buddyDecisions";
import {
  BuddyPresencePreviewContext,
  buddyWorkStateFromFacts,
} from "./buddyPresenceMotion";

/** Mounted only by the explicit fixture opt-in; never a live rollout provider. */
export function BuddyPresencePreviewProvider({
  children,
  connected = false,
  lowPower = false,
  textOnly = false,
}: {
  children: ReactNode;
  connected?: boolean;
  lowPower?: boolean;
  textOnly?: boolean;
}) {
  const identity = useAppSelector(buddyOperatorIdentity);
  const visible = useIsDocumentVisible();
  const options = {
    skip: !connected,
    pollingInterval: visible ? 5000 : 0,
    refetchOnFocus: true,
    refetchOnMountOrArgChange: true,
  };
  const work = useGetBuddyOperatorWorkQuery(
    { endpoint_identity: identity, limit: 50 },
    options,
  );
  const native = useGetBuddyNativeWorkQuery(
    { endpoint_identity: identity },
    options,
  );
  const questions = useGetBuddyNativeDecisionsQuery(
    { endpoint_identity: identity },
    options,
  );
  // Never reuse another endpoint/project, fetching or failed status as live work.
  const current =
    !work.error &&
    !native.error &&
    !questions.error &&
    !work.isFetching &&
    !native.isFetching &&
    !questions.isFetching &&
    work.currentData &&
    native.currentData &&
    questions.currentData &&
    work.currentData.project.canonical_root ===
      native.currentData.project_root &&
    native.currentData.project_root === questions.currentData.project_root;
  const workState = current
    ? buddyWorkStateFromFacts(
        work.currentData?.data.map((row) => row.value) ?? [],
        native.currentData?.sessions ?? [],
        questions.currentData?.questions.some(
          (row) => row.question.status === "pending",
        ),
      )
    : "unavailable";
  return (
    <BuddyPresencePreviewContext.Provider
      value={{
        enabled: true,
        lowPower,
        textOnly,
        reducedMotion: undefined,
        workState,
      }}
    >
      {connected && (
        <p>
          {current
            ? "Presence reflects this page of saved work and loaded native sessions only."
            : "Current work presence unavailable; no execution is inferred."}
        </p>
      )}
      {children}
    </BuddyPresencePreviewContext.Provider>
  );
}
