import React from "react";
import { StatisticView } from "../../../components/StatisticView/StatisticView";
import { useGetStatisticDataQuery } from "../../../hooks";

export const ImpactTab: React.FC = () => {
  const state = useGetStatisticDataQuery();
  return (
    <StatisticView
      statisticData={state.data}
      isLoading={state.isLoading}
      error={state.error ? "Error fetching statistics" : ""}
    />
  );
};
