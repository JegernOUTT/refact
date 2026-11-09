import { useGetReviewDeliveriesQuery } from "../../../services/refact/review";

export function usePendingDeliveries(
  reviewId: string,
  enabled: boolean,
): ReadonlySet<string> {
  const { data } = useGetReviewDeliveriesQuery(
    { reviewId },
    { skip: !enabled },
  );
  const pending = new Set(
    (data ?? []).filter((d) => !d.delivered).map((d) => d.contribution_id),
  );
  useGetReviewDeliveriesQuery(
    { reviewId },
    { skip: !enabled || pending.size === 0, pollingInterval: 3000 },
  );
  return pending;
}
