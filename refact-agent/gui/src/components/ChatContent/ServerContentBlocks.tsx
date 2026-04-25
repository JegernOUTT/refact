import React, { useMemo } from "react";
import { useStoredOpen } from "./useStoredOpen";
import { MagnifyingGlassIcon } from "@radix-ui/react-icons";
import { Box, Flex, Text } from "@radix-ui/themes";
import { Link } from "../Link";
import { ToolCard } from "./ToolCard";
import styles from "./ToolCard/OpenAIResponsesTool.module.css";
import scrollbarStyles from "../shared/scrollbar.module.css";

type ServerToolUse = {
  type: "server_tool_use";
  id: string;
  name: string;
  input?: Record<string, unknown>;
};

type WebSearchResult = {
  type: "web_search_result";
  title?: string;
  url?: string;
  encrypted_content?: string;
  page_age?: string | null;
};

type WebSearchToolResult = {
  type: "web_search_tool_result";
  tool_use_id: string;
  content?: WebSearchResult[];
};

type ServerBlock =
  | ServerToolUse
  | WebSearchToolResult
  | Record<string, unknown>;

function isServerToolUse(block: ServerBlock): block is ServerToolUse {
  return "type" in block && block.type === "server_tool_use";
}

function isWebSearchToolResult(
  block: ServerBlock,
): block is WebSearchToolResult {
  return "type" in block && block.type === "web_search_tool_result";
}

function isSafeHttpUrl(url: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}

type WebSearchGroup = {
  toolUse: ServerToolUse;
  result?: WebSearchToolResult;
};

function groupServerBlocks(blocks: unknown[]): {
  webSearchGroups: WebSearchGroup[];
  ungrouped: unknown[];
} {
  const typedBlocks = blocks as ServerBlock[];
  const webSearchGroups: WebSearchGroup[] = [];
  const grouped = new Set<number>();

  for (let i = 0; i < typedBlocks.length; i++) {
    const block = typedBlocks[i];
    if (isServerToolUse(block) && block.name === "web_search") {
      const resultIdx = typedBlocks.findIndex(
        (b, j) =>
          j > i && isWebSearchToolResult(b) && b.tool_use_id === block.id,
      );
      const group: WebSearchGroup = { toolUse: block };
      grouped.add(i);
      if (resultIdx >= 0) {
        group.result = typedBlocks[resultIdx] as WebSearchToolResult;
        grouped.add(resultIdx);
      }
      webSearchGroups.push(group);
    }
  }

  const ungrouped = typedBlocks.filter((_, i) => !grouped.has(i));
  return { webSearchGroups, ungrouped };
}

const WebSearchBlock: React.FC<{ group: WebSearchGroup }> = ({ group }) => {
  const storeKey = group.toolUse.id ? `srvweb:${group.toolUse.id}` : undefined;
  const [isOpen, toggleOpen] = useStoredOpen(storeKey, false);

  const query =
    typeof group.toolUse.input?.query === "string"
      ? group.toolUse.input.query
      : undefined;

  const results = useMemo(() => {
    if (!group.result?.content) return [];
    if (!Array.isArray(group.result.content)) return [];
    return group.result.content.slice(0, 50);
  }, [group.result]);

  const summary = query ? (
    <>
      Web Search: <span className={styles.inlineCode}>{query}</span>
    </>
  ) : (
    "Web Search"
  );

  return (
    <ToolCard
      icon={<MagnifyingGlassIcon />}
      summary={summary}
      status="success"
      isOpen={isOpen}
      onToggle={toggleOpen}
    >
      {results.length > 0 && (
        <Box>
          <Text size="1" color="gray">
            Results ({results.length})
          </Text>
          <Box className={styles.resultList}>
            {results.map((r, idx) => {
              const title = r.title ?? "(no title)";
              const url = r.url ?? "";
              const safeUrl = url && isSafeHttpUrl(url) ? url : "";
              return (
                <Box key={idx} className={styles.resultItem}>
                  <Flex direction="column" gap="1">
                    {safeUrl ? (
                      <Link
                        href={safeUrl}
                        target="_blank"
                        rel="noopener noreferrer"
                        size="2"
                      >
                        {title}
                      </Link>
                    ) : (
                      <Text size="2" weight="medium">
                        {title}
                      </Text>
                    )}
                    {safeUrl && (
                      <Text size="1" color="gray" className={styles.inlineCode}>
                        {safeUrl}
                      </Text>
                    )}
                  </Flex>
                </Box>
              );
            })}
          </Box>
        </Box>
      )}
      {results.length === 0 && !group.result && (
        <Text size="1" color="gray">
          Waiting for results…
        </Text>
      )}
    </ToolCard>
  );
};

type ServerContentBlocksProps = {
  blocks: unknown[];
};

export const ServerContentBlocks: React.FC<ServerContentBlocksProps> = ({
  blocks,
}) => {
  const { webSearchGroups, ungrouped } = useMemo(
    () => groupServerBlocks(blocks),
    [blocks],
  );

  if (webSearchGroups.length === 0 && ungrouped.length === 0) return null;

  return (
    <Box>
      {webSearchGroups.map((group) => (
        <WebSearchBlock key={group.toolUse.id} group={group} />
      ))}
      {ungrouped.length > 0 && (
        <Box mt="2">
          <Text size="1" color="gray">
            Server blocks ({ungrouped.length})
          </Text>
          <pre
            style={{
              fontSize: "var(--font-size-1)",
              color: "var(--gray-11)",
              overflowX: "auto",
              maxHeight: 200,
            }}
            className={scrollbarStyles.scrollbarThin}
          >
            {JSON.stringify(ungrouped, null, 2)}
          </pre>
        </Box>
      )}
    </Box>
  );
};
