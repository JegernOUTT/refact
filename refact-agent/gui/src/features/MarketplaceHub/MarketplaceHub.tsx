import React from "react";
import { Button, Flex, Heading, Text } from "@radix-ui/themes";
import {
  ArrowLeftIcon,
  ArrowRightIcon,
  CubeIcon,
  FileTextIcon,
  LightningBoltIcon,
  PersonIcon,
} from "@radix-ui/react-icons";
import { PageWrapper } from "../../components/PageWrapper";
import { ScrollArea } from "../../components/ScrollArea";
import { useAppDispatch } from "../../hooks";
import { push } from "../Pages/pagesSlice";
import type { Config } from "../Config/configSlice";
import styles from "./MarketplaceHub.module.css";

type MarketplaceHubProps = {
  host: Config["host"];
  tabbed: Config["tabbed"];
  back: () => void;
};

type HubCard = {
  icon: React.ReactNode;
  title: string;
  description: string;
  action: () => void;
};

const ICON_SIZE = 22;

export const MarketplaceHub: React.FC<MarketplaceHubProps> = ({
  host,
  back,
}) => {
  const dispatch = useAppDispatch();

  const cards: HubCard[] = [
    {
      icon: <LightningBoltIcon width={ICON_SIZE} height={ICON_SIZE} />,
      title: "Skills",
      description:
        "Agent skills that run automatically during coding sessions — code review, brainstorming, security checks, and more.",
      action: () => dispatch(push({ name: "skills marketplace" })),
    },
    {
      icon: <FileTextIcon width={ICON_SIZE} height={ICON_SIZE} />,
      title: "Commands",
      description:
        "Slash commands you invoke explicitly — /review, /test-plan, /commit-message, and hundreds more.",
      action: () => dispatch(push({ name: "commands marketplace" })),
    },
    {
      icon: <PersonIcon width={ICON_SIZE} height={ICON_SIZE} />,
      title: "Subagents",
      description:
        "Specialized sub-agents that handle complex multi-step tasks — SDLC workflows, DevOps, research, and domain-specific automation.",
      action: () => dispatch(push({ name: "subagents marketplace" })),
    },
    {
      icon: <CubeIcon width={ICON_SIZE} height={ICON_SIZE} />,
      title: "MCP Servers",
      description:
        "Model Context Protocol servers that extend the agent with external tools — GitHub, Playwright, Notion, Slack, databases, and more.",
      action: () => dispatch(push({ name: "mcp marketplace" })),
    },
  ];

  return (
    <PageWrapper host={host} style={{ padding: "var(--space-4)" }}>
      <ScrollArea scrollbars="vertical" fullHeight>
        <Flex direction="column" gap="4">
          <Flex align="center" gap="3">
            <Button variant="ghost" size="1" onClick={back}>
              <ArrowLeftIcon />
              Back
            </Button>
            <Heading size="4">Marketplace</Heading>
          </Flex>

          <Text size="2" color="gray">
            Browse and install extensions for Refact. Each category is backed by
            curated community sources — enable a source once, then install
            individual items into your project or global config.
          </Text>

          <div className={styles.grid}>
            {cards.map((card) => (
              <button
                key={card.title}
                className={styles.card}
                onClick={card.action}
                type="button"
              >
                <Flex direction="column" gap="2" className={styles.cardBody}>
                  <Flex align="center" gap="2" className={styles.cardHeader}>
                    <span className={styles.cardIcon}>{card.icon}</span>
                    <Text size="3" weight="bold">
                      {card.title}
                    </Text>
                    <span className={styles.cardArrow}>
                      <ArrowRightIcon width={14} height={14} />
                    </span>
                  </Flex>
                  <Text size="2" color="gray" className={styles.cardDesc}>
                    {card.description}
                  </Text>
                </Flex>
              </button>
            ))}
          </div>
        </Flex>
      </ScrollArea>
    </PageWrapper>
  );
};
