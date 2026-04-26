import React from "react";
import { useGetExtRegistryQuery } from "../../services/refact/extensions";
import {
  useGetSkillsMarketplaceQuery,
  useInstallMarketplaceSkillMutation,
  type ExtensionMarketplaceItem,
} from "../../services/refact/extensionsMarketplace";
import type { Config } from "../Config/configSlice";
import { ExtensionsMarketplace } from "../ExtensionsMarketplace";

type SkillsMarketplaceProps = {
  host: Config["host"];
  tabbed: Config["tabbed"];
  backFromMarketplace: () => void;
};

export const SkillsMarketplace: React.FC<SkillsMarketplaceProps> = ({
  host,
  tabbed,
  backFromMarketplace,
}) => {
  const { data: registry } = useGetExtRegistryQuery(undefined);
  const { data, isLoading, error } = useGetSkillsMarketplaceQuery(undefined);
  const [installSkill, { isLoading: isInstalling }] =
    useInstallMarketplaceSkillMutation();

  const hasProjectRoot = registry?.has_project_root ?? false;

  return (
    <ExtensionsMarketplace
      host={host}
      tabbed={tabbed}
      title="Skills Marketplace"
      kind="skill"
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
        await installSkill({
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
