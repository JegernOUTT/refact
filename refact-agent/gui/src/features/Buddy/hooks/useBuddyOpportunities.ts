import { useAppSelector } from "../../../hooks";
import { useGetOpportunitiesQuery } from "../../../services/refact/buddy";
import { selectUnreadOpportunities, selectOpportunities } from "../buddySlice";

export function useBuddyOpportunities() {
  const { isLoading } = useGetOpportunitiesQuery(undefined, {
    refetchOnMountOrArgChange: true,
  });
  const opportunities = useAppSelector(selectOpportunities);
  const unread = useAppSelector(selectUnreadOpportunities);
  return { opportunities, unread, isLoading };
}
