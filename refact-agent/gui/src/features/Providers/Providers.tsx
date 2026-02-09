import React from "react";
import { Flex } from "@radix-ui/themes";

import { ScrollArea } from "../../components/ScrollArea";
import { PageWrapper } from "../../components/PageWrapper";
import { Spinner } from "../../components/Spinner";
import { ProvidersView } from "./ProvidersView";
import { ProviderUpdateProvider } from "./ProviderUpdateContext";

import { useGetConfiguredProvidersQuery } from "../../hooks/useProvidersQuery";

import type { Config } from "../Config/configSlice";

export type ProvidersProps = {
  backFromProviders: () => void;
  host: Config["host"];
  tabbed: Config["tabbed"];
};
export const Providers: React.FC<ProvidersProps> = ({
  backFromProviders,
  host,
}) => {
  const { data: configuredProvidersData, isSuccess } =
    useGetConfiguredProvidersQuery();

  if (!isSuccess) return <Spinner spinning />;
  return (
    <PageWrapper
      host={host}
      style={{
        padding: 0,
        marginTop: 0,
      }}
    >
      <ScrollArea scrollbars="vertical" fullHeight>
        <Flex
          direction="column"
          justify="between"
          flexGrow="1"
          style={{
            width: "inherit",
            height: "100%",
          }}
        >
          <ProviderUpdateProvider>
            <ProvidersView
              configuredProviders={configuredProvidersData.providers}
              backFromProviders={backFromProviders}
            />
          </ProviderUpdateProvider>
        </Flex>
      </ScrollArea>
    </PageWrapper>
  );
};
