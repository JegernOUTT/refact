import { useEffect, useRef, useState } from "react";
import { useAppSelector } from "../../hooks";
import { useIsDocumentVisible } from "../../hooks/useIsDocumentVisible";
import { buddyOperatorIdentity } from "../../services/refact/buddyOperator";
import {
  useClaimBuddyEditorialMutation,
  useFeedbackBuddyEditorialMutation,
  type EditorialFeedback,
} from "../../services/refact/buddyEditorial";

import { useGetBuddySupervisionContextQuery } from "../../services/refact/buddySupervision";

export function requiresEditorialClaim(value: unknown): boolean {
  return (
    !!value &&
    typeof value === "object" &&
    "editorial_delivery_required" in value &&
    value.editorial_delivery_required === true
  );
}

/** Avatar delivery stays closed. Only the mounted, visible text surface may claim. */
export function useBuddyEditorialDelivery<T extends { id: string }>(
  candidate: T | null | undefined,
  surface: "panel" | "avatar",
  enabled = true,
) {
  const identity = useAppSelector(buddyOperatorIdentity);
  const visible = useIsDocumentVisible();
  const contextQuery = useGetBuddySupervisionContextQuery(
    { endpoint_identity: identity },
    {
      skip:
        !requiresEditorialClaim(candidate) || surface === "avatar" || !enabled,
      pollingInterval: visible ? 5000 : 0,
      skipPollingIfUnfocused: true,
      refetchOnMountOrArgChange: true,
    },
  );
  const context =
    contextQuery.error || contextQuery.isFetching
      ? undefined
      : contextQuery.currentData;
  const project = context?.project;
  const worker = context?.worker;
  const [claim] = useClaimBuddyEditorialMutation();
  const [feedback] = useFeedbackBuddyEditorialMutation();
  const key = JSON.stringify([identity, candidate, surface, project, worker]);
  const current = useRef(key);
  current.current = key;
  // Never evict uncertain claims: eviction would silently enable replay. At the
  // session cap, withhold new deliveries until this surface is remounted.
  const pending = useRef(
    new Map<string, { key: string; request: Promise<boolean> }>(),
  );
  const claimKey = JSON.stringify([identity, candidate?.id, project, worker]);
  const [delivered, setDelivered] = useState<string | null>(null);
  const [feedbackState, setFeedbackState] = useState<{
    key: string;
    text: string;
  } | null>(null);
  const feedbackLock = useRef(false);
  const gated = requiresEditorialClaim(candidate);
  useEffect(() => {
    if (
      !candidate ||
      !gated ||
      !visible ||
      !enabled ||
      surface === "avatar" ||
      !project ||
      !worker ||
      !context?.features.editorial
    )
      return;
    let active = true;
    let entry = pending.current.get(claimKey);
    if (!entry) {
      if (pending.current.size >= 128) return;
      const request = claim({
        endpoint_identity: identity,
        project,
        worker,
        speech_id: candidate.id,
        surface: "panel",
      }).unwrap();
      entry = { key, request };
      pending.current.set(claimKey, entry);
    }
    if (entry.key !== key) return;
    void entry.request
      .then((claimed) => {
        if (active && current.current === key && claimed) setDelivered(key);
      })
      .catch(() => {
        /* Fail closed; never retry an uncertain delivery automatically. */
      });
    return () => {
      active = false;
    };
  }, [
    candidate,
    claim,
    claimKey,
    enabled,
    gated,
    identity,
    key,
    surface,
    visible,
    project,
    worker,
    context?.features.editorial,
  ]);
  const speech =
    !enabled || !visible
      ? null
      : gated
        ? surface === "panel" &&
          context?.features.editorial &&
          delivered === key
          ? candidate
          : null
        : candidate;
  const sendFeedback = async (choice: EditorialFeedback) => {
    if (
      !speech ||
      !gated ||
      delivered !== key ||
      feedbackLock.current ||
      !project ||
      !worker
    )
      return;
    feedbackLock.current = true;
    setFeedbackState({ key, text: "Sending feedback…" });
    try {
      const recorded = await feedback({
        endpoint_identity: identity,
        project,
        worker,
        speech_id: speech.id,
        feedback: choice,
      }).unwrap();
      if (current.current === key)
        setFeedbackState({
          key,
          text: recorded
            ? "Feedback recorded. Permissions unchanged."
            : "Feedback not recorded.",
        });
    } catch {
      if (current.current === key)
        setFeedbackState({
          key,
          text: "Feedback unconfirmed. Permissions unchanged.",
        });
    } finally {
      feedbackLock.current = false;
    }
  };
  return {
    speech,
    canFeedback: !!speech && gated && delivered === key,
    sendFeedback,
    feedbackStatus: feedbackState?.key === key ? feedbackState.text : null,
  };
}
