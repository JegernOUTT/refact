import React from "react";
import { Container, Box, Text, Flex } from "@radix-ui/themes";
import * as Collapsible from "@radix-ui/react-collapsible";
import { Chevron } from "../Collapsible";
import { Markdown } from "./ContextFiles";

export const SystemPrompt: React.FC<{
  content: string;
}> = ({ content }) => {
  const [open, setOpen] = React.useState(false);

  if (!content.trim()) return null;

  return (
    <Container>
      <Collapsible.Root open={open} onOpenChange={setOpen}>
        <Collapsible.Trigger asChild>
          <Flex gap="2" align="center" py="2" style={{ cursor: "pointer" }}>
            <Text weight="light" size="1" style={{ color: "var(--gray-10)" }}>
              📋 System prompt
            </Text>
            <Chevron open={open} />
          </Flex>
        </Collapsible.Trigger>
        <Collapsible.Content>
          <Box
            pl="2"
            style={{
              borderLeft: "1px solid var(--gray-a4)",
              maxHeight: "400px",
              overflow: "auto",
            }}
          >
            <Markdown>{content}</Markdown>
          </Box>
        </Collapsible.Content>
      </Collapsible.Root>
    </Container>
  );
};
