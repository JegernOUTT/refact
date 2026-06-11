import React from "react";
import { ArrowLeft } from "lucide-react";
import { PageWrapper } from "../../components/PageWrapper";
import { ScrollArea } from "../../components/ScrollArea";
import { Button, Tabs } from "../../components/ui";
import { useAppDispatch } from "../../hooks";
import { CommandsMarketplace } from "../CommandsMarketplace";
import type { Config } from "../Config/configSlice";
import { MarketplacePanel } from "../Extensions/components/MarketplacePanel";
import { MCPMarketplace } from "../MCPMarketplace";
import { change } from "../Pages/pagesSlice";
import type { Page } from "../Pages/pagesSlice";
import { SkillsMarketplace } from "../SkillsMarketplace";
import { SettingsSection } from "../Settings/SettingsSection";
import { SubagentsMarketplace } from "../SubagentsMarketplace";
import {
  marketplacePageToTab,
  marketplaceTabToPage,
  type MarketplaceTabId,
} from "./marketplaceRoutes";
import styles from "./MarketplaceHub.module.css";

type MarketplaceHubProps = {
  host: Config["host"];
  tabbed: Config["tabbed"];
  back: () => void;
  page: Page;
  embedded?: boolean;
};

const MARKETPLACE_DESCRIPTION =
  "Browse and install skills, commands, subagents, MCP servers, and extension plugins from curated community sources.";

const tabs: { id: MarketplaceTabId; label: string }[] = [
  { id: "skills", label: "Skills" },
  { id: "commands", label: "Commands" },
  { id: "subagents", label: "Subagents" },
  { id: "mcp", label: "MCP Servers" },
  { id: "extensions", label: "Extensions" },
];

export const MarketplaceHub: React.FC<MarketplaceHubProps> = ({
  host,
  tabbed,
  back,
  page,
  embedded = false,
}) => {
  const dispatch = useAppDispatch();
  const activeTab = marketplacePageToTab(page) ?? "skills";
  const activeIndex = tabs.findIndex((tab) => tab.id === activeTab);

  const handleTabChange = (next: string) => {
    dispatch(change(marketplaceTabToPage(next as MarketplaceTabId)));
  };

  const tabsList = (
    <Tabs.List
      activeIndex={Math.max(activeIndex, 0)}
      itemCount={tabs.length}
      className={styles.tabsList}
    >
      {tabs.map((tab) => (
        <Tabs.Trigger key={tab.id} value={tab.id}>
          {tab.label}
        </Tabs.Trigger>
      ))}
    </Tabs.List>
  );

  const panels = (
    <>
      <Tabs.Content value="skills" className={styles.tabPanel}>
        {activeTab === "skills" && (
          <SkillsMarketplace
            embedded
            host={host}
            tabbed={tabbed}
            backFromMarketplace={back}
          />
        )}
      </Tabs.Content>
      <Tabs.Content value="commands" className={styles.tabPanel}>
        {activeTab === "commands" && (
          <CommandsMarketplace
            embedded
            host={host}
            tabbed={tabbed}
            backFromMarketplace={back}
          />
        )}
      </Tabs.Content>
      <Tabs.Content value="subagents" className={styles.tabPanel}>
        {activeTab === "subagents" && (
          <SubagentsMarketplace
            embedded
            host={host}
            tabbed={tabbed}
            backFromMarketplace={back}
          />
        )}
      </Tabs.Content>
      <Tabs.Content value="mcp" className={styles.tabPanel}>
        {activeTab === "mcp" && (
          <MCPMarketplace
            embedded
            host={host}
            tabbed={tabbed}
            backFromMarketplace={back}
          />
        )}
      </Tabs.Content>
      <Tabs.Content value="extensions" className={styles.tabPanel}>
        {activeTab === "extensions" && <MarketplacePanel />}
      </Tabs.Content>
    </>
  );

  if (embedded) {
    return (
      <Tabs value={activeTab} onValueChange={handleTabChange}>
        <SettingsSection
          title="Marketplace"
          description={MARKETPLACE_DESCRIPTION}
          subNav={tabsList}
        >
          {panels}
        </SettingsSection>
      </Tabs>
    );
  }

  return (
    <PageWrapper host={host}>
      <ScrollArea scrollbars="vertical" fullHeight>
        <div className={styles.pageStack}>
          <div className={styles.header}>
            <Button
              variant="ghost"
              size="sm"
              leftIcon={ArrowLeft}
              onClick={back}
            >
              Back
            </Button>
            <div className={styles.headerText}>
              <h2 className={styles.title}>Marketplace</h2>
              <p className={styles.description}>{MARKETPLACE_DESCRIPTION}</p>
            </div>
          </div>

          <Tabs
            value={activeTab}
            onValueChange={handleTabChange}
            className={styles.tabsRoot}
          >
            {tabsList}
            {panels}
          </Tabs>
        </div>
      </ScrollArea>
    </PageWrapper>
  );
};
