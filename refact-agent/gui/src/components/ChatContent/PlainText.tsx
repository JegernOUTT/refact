import React from "react";
import { useStoredOpen } from "./useStoredOpen";
import { Container, Box, Flex, Text } from "@radix-ui/themes";
import { ChevronDown, ChevronRight, FileText } from "lucide-react";
import { Markdown } from "./ContextFiles";
import styles from "./ChatContent.module.css";
import { ScrollArea } from "../ScrollArea";
import { Icon } from "../ui";
import * as Collapsible from "@radix-ui/react-collapsible";

export type PlainTextProps = {
  children: string;
  id?: string;
  defaultOpen?: boolean;
};

export const PlainText: React.FC<PlainTextProps> = ({
  children,
  id,
  defaultOpen = false,
}) => {
  const storeKey = id ? `plaintext:${id}` : undefined;
  const [open, _toggleOpen, setOpen] = useStoredOpen(storeKey, defaultOpen);
  const text = "```text\n" + children + "\n```";
  const preview =
    children.slice(0, 100).replace(/\n/g, " ") +
    (children.length > 100 ? "..." : "");
  const ChevronIcon = open ? ChevronDown : ChevronRight;

  return (
    <Container position="relative" data-plain-text-id={id}>
      <Collapsible.Root open={open} onOpenChange={setOpen}>
        <Collapsible.Trigger asChild>
          <Flex
            gap="2"
            align="center"
            py="1"
            className={`${styles.plainTextTrigger} rf-pressable`}
          >
            <Icon icon={FileText} size="sm" tone="muted" />
            <Text size="1" weight="light" className={styles.plainTextLabel}>
              Plain text
            </Text>
            <Text size="1" className={styles.plainTextPreview} truncate>
              {preview}
            </Text>
            <Icon icon={ChevronIcon} size="sm" tone="muted" />
          </Flex>
        </Collapsible.Trigger>
        <Collapsible.Content className="rf-expand-grid">
          <ScrollArea scrollbars="both">
            <Box className={styles.plainTextBody}>
              <Markdown>{text}</Markdown>
            </Box>
          </ScrollArea>
        </Collapsible.Content>
      </Collapsible.Root>
    </Container>
  );
};
