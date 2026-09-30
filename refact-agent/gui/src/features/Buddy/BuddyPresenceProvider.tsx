import { useMemo, useRef, type ReactNode } from "react";
import { useAppSelector } from "../../hooks";
import { useIsDocumentVisible } from "../../hooks/useIsDocumentVisible";
import { buddyOperatorIdentity } from "../../services/refact/buddyOperator";
import { useGetBuddyNativeWorkQuery } from "../../services/refact/buddyNativeWork";
import { useGetBuddyNativeDecisionsQuery } from "../../services/refact/buddyDecisions";
import {
  BuddyPresencePreviewContext,
  buddyWorkStateFromFacts,
} from "./buddyPresenceMotion";
import { useBuddyTextPresence } from "./useBuddyTextPresence";

const PRESENCE_POLLING_INTERVAL_MS = 5000;

export function BuddyPresenceProvider({
  children,
  lowPower = false,
}: {
  children: ReactNode;
  lowPower?: boolean;
}) {
  const identity = useAppSelector(buddyOperatorIdentity);
  const refetchRef = useRef<() => void>(() => undefined);
  const visible = useIsDocumentVisible(() => refetchRef.current());
  const [textOnly] = useBuddyTextPresence();
  const options = {
    pollingInterval: visible ? PRESENCE_POLLING_INTERVAL_MS : 0,
    skipPollingIfUnfocused: true,
    refetchOnFocus: true,
    refetchOnMountOrArgChange: true,
  };
  const native = useGetBuddyNativeWorkQuery(
    { endpoint_identity: identity },
    options,
  );
  const questions = useGetBuddyNativeDecisionsQuery(
    { endpoint_identity: identity },
    options,
  );
  refetchRef.current = () => {
    void native.refetch();
    void questions.refetch();
  };
  const sessions = native.currentData?.sessions;
  const pendingQuestion = questions.currentData?.questions.some(
    (row) => row.question.status === "pending",
  );
  const sameProject =
    native.currentData !== undefined &&
    questions.currentData !== undefined &&
    native.currentData.project_root === questions.currentData.project_root;
  const live =
    sameProject && !native.error && !questions.error && sessions !== undefined;
  const workState = live
    ? buddyWorkStateFromFacts([], sessions, pendingQuestion)
    : "unavailable";
  const value = useMemo(
    () => ({
      enabled: true,
      lowPower,
      textOnly,
      reducedMotion: undefined,
      workState,
    }),
    [lowPower, textOnly, workState],
  );
  return (
    <BuddyPresencePreviewContext.Provider value={value}>
      {children}
    </BuddyPresencePreviewContext.Provider>
  );
}
