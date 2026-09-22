import { useAppSelector } from "../../../hooks";
import { useGetOpportunitiesQuery } from "../../../services/refact/buddy";
import { selectUnreadOpportunities, selectOpportunities } from "../buddySlice";
import {
  buddyOperatorIdentity,
  useGetBuddyOperatorStatusQuery,
} from "../../../services/refact/buddyOperator";

export function useBuddyOpportunities() {
  const identity = useAppSelector(buddyOperatorIdentity);
  const authority = useGetBuddyOperatorStatusQuery(identity);
  const compatibility =
    !authority.error && authority.currentData?.mode === "disabled";
  const { isLoading } = useGetOpportunitiesQuery(undefined, {
    skip: !compatibility,
    refetchOnMountOrArgChange: true,
  });
  const opportunities = useAppSelector(selectOpportunities);
  const unread = useAppSelector(selectUnreadOpportunities);
  return {
    opportunities: compatibility ? opportunities : [],
    unread: compatibility ? unread : [],
    isLoading: authority.isFetching || isLoading,
  };
}
