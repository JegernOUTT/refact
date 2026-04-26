import React from "react";
import { useGetExtRegistryQuery } from "../../services/refact/extensions";
import {
  useGetCommandsMarketplaceQuery,
  useInstallMarketplaceCommandMutation,
  type ExtensionMarketplaceItem,
} from "../../services/refact/extensionsMarketplace";
import type { Config } from "../Config/configSlice";
import { ExtensionsMarketplace } from "../ExtensionsMarketplace";

type CommandsMarketplaceProps = {
  host: Config["host"];
  tabbed: Config["tabbed"];
  backFromMarketplace: () => void;
};

export const CommandsMarketplace: React.FC<CommandsMarketplaceProps> = ({
  host,
  tabbed,
  backFromMarketplace,
}) => {
  const { data: registry } = useGetExtRegistryQuery(undefined);
  const { data, isLoading, error } = useGetCommandsMarketplaceQuery(undefined);
  const [installCommand, { isLoading: isInstalling }] =
    useInstallMarketplaceCommandMutation();

  const hasProjectRoot = registry?.has_project_root ?? false;

  return (
    <ExtensionsMarketplace
      host={host}
      tabbed={tabbed}
      title="Commands Marketplace"
      kind="command"
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
        await installCommand({
          source_id: item.source_id,
          item_id: item.id,
          scope,
          params: params ?? {},
          overwrite: overwrite ?? false,
        }).unwrap();
      }}
    />
  );
};
