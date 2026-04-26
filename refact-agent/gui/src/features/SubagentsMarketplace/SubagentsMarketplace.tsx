import React from "react";
import { useGetRegistryQuery } from "../../services/refact/customization";
import {
  type ExtensionMarketplaceItem,
  useGetSubagentsMarketplaceQuery,
  useInstallMarketplaceSubagentMutation,
} from "../../services/refact/extensionsMarketplace";
import type { Config } from "../Config/configSlice";
import { ExtensionsMarketplace } from "../ExtensionsMarketplace";

type SubagentsMarketplaceProps = {
  host: Config["host"];
  tabbed: Config["tabbed"];
  backFromMarketplace: () => void;
};

export const SubagentsMarketplace: React.FC<SubagentsMarketplaceProps> = ({
  host,
  tabbed,
  backFromMarketplace,
}) => {
  const { data: registry } = useGetRegistryQuery(undefined);
  const { data, isLoading, error } = useGetSubagentsMarketplaceQuery(undefined);
  const [installSubagent, { isLoading: isInstalling }] =
    useInstallMarketplaceSubagentMutation();

  const hasProjectRoot = registry?.has_project_root ?? false;

  return (
    <ExtensionsMarketplace
      host={host}
      tabbed={tabbed}
      title="Subagents Marketplace"
      kind="subagent"
      back={backFromMarketplace}
      items={data?.items ?? []}
      sources={data?.sources ?? []}
      isLoading={isLoading}
      error={error}
      isInstalling={isInstalling}
      hasProjectRoot={hasProjectRoot}
      onInstall={async (
        item: ExtensionMarketplaceItem,
        scope,
        params,
        overwrite,
      ) => {
        await installSubagent({
          source_id: item.source_id,
          item_id: item.id,
          scope,
          params: params ?? {},
          overwrite: overwrite ?? false,
        }).unwrap();
      }}
      onInstalled={(item) => ({
        name: "customization",
        kind: "subagents",
        configId: item.id,
      })}
    />
  );
};
