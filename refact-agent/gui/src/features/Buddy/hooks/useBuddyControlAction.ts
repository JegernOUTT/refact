import { useCallback, useEffect, useRef, useState } from "react";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import {
  executeBuddyAction,
  type BuddyActionContext,
  type BuddyActionOutcome,
} from "../executeBuddyAction";
import {
  BUDDY_ACTION_SUCCESS,
  formatOpportunityActionError,
  useExecuteBuddyAction,
  type BuddyControlActionResult,
} from "./useExecuteBuddyAction";
import type { BuddyAction, BuddyControl, BuddyOpportunity } from "../types";

export type { BuddyControlActionResult } from "./useExecuteBuddyAction";

const DUPLICATE: BuddyControlActionResult = {
  status: "ignored",
  reason: "duplicate",
};
const STALE: BuddyControlActionResult = { status: "ignored", reason: "stale" };

function fromLegacyOutcome(
  outcome: BuddyActionOutcome,
): BuddyControlActionResult {
  if (outcome.status === "ok") return BUDDY_ACTION_SUCCESS;
  return outcome;
}

export function useBuddyControlAction(ownerKey = "") {
  const dispatch = useAppDispatch();
  const executeOpportunity = useExecuteBuddyAction();
  const projectIdentity = useAppSelector((state) =>
    JSON.stringify({
      name: state.current_project.name,
      workspaceRoots: state.current_project.workspaceRoots,
      lspPort: state.config.lspPort,
      lspUrl: state.config.lspUrl,
      ownerKey,
    }),
  );

  const inFlight = useRef(new Map<string, symbol>());
  const mounted = useRef(true);
  const epoch = useRef(0);
  const identity = useRef(projectIdentity);
  identity.current = projectIdentity;
  const [pendingActionIds, setPendingActionIds] = useState<Set<string>>(
    () => new Set(),
  );

  useEffect(() => {
    mounted.current = true;
    const requests = inFlight.current;
    return () => {
      mounted.current = false;
      epoch.current += 1;
      requests.clear();
    };
  }, []);

  useEffect(() => {
    epoch.current += 1;
    inFlight.current.clear();
    setPendingActionIds((current) =>
      current.size === 0 ? current : new Set(),
    );
  }, [projectIdentity]);

  const run = useCallback(
    async (
      actionId: string,
      operation: (
        isCurrent: () => boolean,
      ) => Promise<BuddyControlActionResult>,
    ): Promise<BuddyControlActionResult> => {
      if (inFlight.current.has(actionId)) return DUPLICATE;
      const startedEpoch = epoch.current;
      const startedIdentity = identity.current;
      const requestToken = Symbol(actionId);
      const isCurrent = () =>
        mounted.current &&
        epoch.current === startedEpoch &&
        identity.current === startedIdentity &&
        inFlight.current.get(actionId) === requestToken;
      inFlight.current.set(actionId, requestToken);
      setPendingActionIds((current) => new Set(current).add(actionId));

      let result: BuddyControlActionResult;
      let current = false;
      try {
        result = await operation(isCurrent);
      } catch (error) {
        result = {
          status: "failed",
          message: formatOpportunityActionError(error),
        };
      } finally {
        current = isCurrent();
        if (inFlight.current.get(actionId) === requestToken) {
          inFlight.current.delete(actionId);
        }
        if (current) {
          setPendingActionIds((current) => {
            if (!current.has(actionId)) return current;
            const next = new Set(current);
            next.delete(actionId);
            return next;
          });
        }
      }

      if (!current) return STALE;
      return result;
    },
    [],
  );

  const control = useCallback(
    (
      ctrl: BuddyControl,
      context?: BuddyActionContext,
      source = "control",
    ): Promise<BuddyControlActionResult> =>
      run(`${source}:${ctrl.id || ctrl.action}`, async (isCurrent) =>
        fromLegacyOutcome(
          await executeBuddyAction(ctrl, dispatch, context, isCurrent),
        ),
      ),
    [dispatch, run],
  );

  const opportunity = useCallback(
    (
      action: BuddyAction,
      opp: BuddyOpportunity | null,
      actionIndex: number,
      options?: { never?: boolean },
      source = "opportunity",
    ): Promise<BuddyControlActionResult> =>
      run(`${source}:${opp?.id ?? "no-origin"}:${actionIndex}`, (isCurrent) =>
        executeOpportunity(action, opp, actionIndex, options, isCurrent),
      ),
    [executeOpportunity, run],
  );

  const isPending = useCallback(
    (actionId: string) => pendingActionIds.has(actionId),
    [pendingActionIds],
  );

  return { control, opportunity, pendingActionIds, isPending };
}
