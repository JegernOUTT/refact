import React from "react";
import { useStoredOpen } from "./useStoredOpen";
import { Container, Box, Flex, Text } from "@radix-ui/themes";
import { FileText } from "lucide-react";
import { Markdown } from "./ContextFiles";
import { AnimatedCollapsible } from "./shared/AnimatedCollapsible";
import styles from "./ChatContent.module.css";
import { ScrollArea } from "../ScrollArea";
import { Icon } from "../ui";

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
  const [open, , setOpen] = useStoredOpen(storeKey, defaultOpen);
  const text = "```text\n" + children + "\n```";
  const preview =
    children.slice(0, 100).replace(/\n/g, " ") +
    (children.length > 100 ? "..." : "");

  return (
    <Container position="relative" data-plain-text-id={id}>
      <AnimatedCollapsible
        className={styles.plainTextCollapsible}
        header={
          <Flex align="center" gap="2" className={styles.plainTextHeader}>
            <Text size="1" weight="light" className={styles.plainTextLabel}>
              Plain text
            </Text>
            <Text size="1" className={styles.plainTextPreview} truncate>
              {preview}
            </Text>
          </Flex>
        }
        icon={<Icon icon={FileText} size="sm" tone="muted" />}
        open={open}
        onOpenChange={setOpen}
        variant="compact"
      >
        <ScrollArea scrollbars="both">
          <Box className={styles.plainTextBody}>
            <Markdown>{text}</Markdown>
          </Box>
        </ScrollArea>
      </AnimatedCollapsible>
    </Container>
  );
};
