import React from "react";

import { ScrollArea } from "../../components/ScrollArea";
import { PageWrapper } from "../../components/PageWrapper";
import { Spinner } from "../../components/Spinner";
import { ProvidersView } from "./ProvidersView";
import styles from "./Providers.module.css";

import { useGetConfiguredProvidersQuery } from "../../hooks/useProvidersQuery";

import type { Config } from "../Config/configSlice";

export type ProvidersProps = {
  backFromProviders: () => void;
  host: Config["host"];
  tabbed: Config["tabbed"];
};
export const Providers: React.FC<ProvidersProps> = ({ backFromProviders, host }) => {
  const { data: configuredProvidersData, isSuccess } = useGetConfiguredProvidersQuery();

  if (!isSuccess) return <Spinner spinning />;
  return (
    <PageWrapper host={host} className={styles.page} noPadding>
      <ScrollArea scrollbars="vertical" fullHeight className={styles.scrollArea}>
        <div className={styles.content}>
          <ProvidersView
            configuredProviders={configuredProvidersData.providers}
            backFromProviders={backFromProviders}
          />
        </div>
      </ScrollArea>
    </PageWrapper>
  );
};
