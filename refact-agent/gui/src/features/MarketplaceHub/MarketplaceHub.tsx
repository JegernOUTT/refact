import React from "react";
import { ArrowLeft, ArrowRight, Box, FileText, User, Zap } from "lucide-react";
import { PageWrapper } from "../../components/PageWrapper";
import { ScrollArea } from "../../components/ScrollArea";
import { Button, Icon, Surface } from "../../components/ui";
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
  icon: React.ComponentProps<typeof Icon>["icon"];
  title: string;
  description: string;
  action: () => void;
};

export const MarketplaceHub: React.FC<MarketplaceHubProps> = ({
  host,
  back,
}) => {
  const dispatch = useAppDispatch();

  const cards: HubCard[] = [
    {
      icon: Zap,
      title: "Skills",
      description:
        "Agent skills that run automatically during coding sessions — code review, brainstorming, security checks, and more.",
      action: () => dispatch(push({ name: "skills marketplace" })),
    },
    {
      icon: FileText,
      title: "Commands",
      description:
        "Slash commands you invoke explicitly — /review, /test-plan, /commit-message, and hundreds more.",
      action: () => dispatch(push({ name: "commands marketplace" })),
    },
    {
      icon: User,
      title: "Subagents",
      description:
        "Specialized sub-agents that handle complex multi-step tasks — SDLC workflows, DevOps, research, and domain-specific automation.",
      action: () => dispatch(push({ name: "subagents marketplace" })),
    },
    {
      icon: Box,
      title: "MCP Servers",
      description:
        "Model Context Protocol servers that extend the agent with external tools — GitHub, Playwright, Notion, Slack, databases, and more.",
      action: () => dispatch(push({ name: "mcp marketplace" })),
    },
  ];

  return (
    <PageWrapper host={host}>
      <ScrollArea scrollbars="vertical" fullHeight>
        <div className={styles.pageStack}>
          <div className={styles.header}>
            <Button variant="ghost" size="sm" leftIcon={ArrowLeft} onClick={back}>
              Back
            </Button>
            <h2 className={styles.title}>Marketplace</h2>
          </div>

          <p className={styles.description}>
            Browse and install extensions for Refact. Each category is backed by
            curated community sources — enable a source once, then install
            individual items into your project or global config.
          </p>

          <div className={styles.grid}>
            {cards.map((card) => (
              <Surface
                as="button"
                animated
                variant="surface-1"
                key={card.title}
                className={styles.card}
                onClick={card.action}
                type="button"
              >
                <div className={styles.cardBody}>
                  <div className={styles.cardHeader}>
                    <span className={styles.cardIcon}>
                      <Icon icon={card.icon} tone="accent" size="lg" />
                    </span>
                    <p className={styles.cardTitle}>{card.title}</p>
                    <span className={styles.cardArrow}>
                      <Icon icon={ArrowRight} tone="muted" size="sm" />
                    </span>
                  </div>
                  <p className={styles.cardDesc}>{card.description}</p>
                </div>
              </Surface>
            ))}
          </div>
        </div>
      </ScrollArea>
    </PageWrapper>
  );
};
