import React, { useState, useCallback } from "react";
import { Flex, Button, Tabs, Text } from "@radix-ui/themes";
import { ArrowLeftIcon } from "@radix-ui/react-icons";

import { PageWrapper } from "../../components/PageWrapper";
import type { Config } from "../Config/configSlice";

import styles from "./Extensions.module.css";

export type ExtensionsTab = "skills" | "commands" | "hooks" | "marketplace";

export type ExtensionsProps = {
  backFromExtensions: () => void;
  host: Config["host"];
  tabbed: Config["tabbed"];
  initialTab?: ExtensionsTab;
  initialItemId?: string;
};

export const Extensions: React.FC<ExtensionsProps> = ({
  backFromExtensions,
  host,
  tabbed,
  initialTab = "skills",
}) => {
  const [activeTab, setActiveTab] = useState<ExtensionsTab>(initialTab);

  const handleTabChange = useCallback((value: string) => {
    setActiveTab(value as ExtensionsTab);
  }, []);

  return (
    <PageWrapper host={host} noPadding>
      {host === "vscode" && !tabbed ? (
        <Flex gap="2" pb="2">
          <Button variant="surface" onClick={backFromExtensions}>
            <ArrowLeftIcon width="16" height="16" />
            Back
          </Button>
        </Flex>
      ) : (
        <Button
          mr="auto"
          variant="outline"
          onClick={backFromExtensions}
          mb="2"
        >
          Back
        </Button>
      )}

      <Tabs.Root value={activeTab} onValueChange={handleTabChange}>
        <Tabs.List size="1">
          <Tabs.Trigger value="skills">Skills</Tabs.Trigger>
          <Tabs.Trigger value="commands">Commands</Tabs.Trigger>
          <Tabs.Trigger value="hooks">Hooks</Tabs.Trigger>
          <Tabs.Trigger value="marketplace">Marketplace</Tabs.Trigger>
        </Tabs.List>

        <div className={styles.panelContainer}>
          <Tabs.Content value="skills">
            <Flex direction="column" className={styles.listPanel}>
              <Text size="2" className={styles.placeholder}>
                Skills editor coming soon.
              </Text>
            </Flex>
          </Tabs.Content>

          <Tabs.Content value="commands">
            <Flex direction="column" className={styles.listPanel}>
              <Text size="2" className={styles.placeholder}>
                Commands editor coming soon.
              </Text>
            </Flex>
          </Tabs.Content>

          <Tabs.Content value="hooks">
            <Flex direction="column" className={styles.listPanel}>
              <Text size="2" className={styles.placeholder}>
                Hooks editor coming soon.
              </Text>
            </Flex>
          </Tabs.Content>

          <Tabs.Content value="marketplace">
            <Flex direction="column" className={styles.listPanel}>
              <Text size="2" className={styles.placeholder}>
                Marketplace coming soon.
              </Text>
            </Flex>
          </Tabs.Content>
        </div>
      </Tabs.Root>
    </PageWrapper>
  );
};
