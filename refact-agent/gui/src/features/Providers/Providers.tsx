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
  embedded?: boolean;
};
export const Providers: React.FC<ProvidersProps> = ({ backFromProviders, host, embedded }) => {
  const { data: configuredProvidersData, isSuccess } = useGetConfiguredProvidersQuery();

  if (!isSuccess) return <Spinner spinning />;

  const content = (
    <ScrollArea scrollbars="vertical" fullHeight className={styles.scrollArea}>
      <div className={styles.content}>
        <ProvidersView
          configuredProviders={configuredProvidersData.providers}
          backFromProviders={backFromProviders}
        />
      </div>
    </ScrollArea>
  );

  if (embedded) {
    return <div className={styles.page}>{content}</div>;
  }

  return (
    <PageWrapper host={host} className={styles.page} noPadding>
      {content}
    </PageWrapper>
  );
};
