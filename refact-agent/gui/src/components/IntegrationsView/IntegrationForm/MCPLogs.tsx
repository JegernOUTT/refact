import React from "react";
import { useGetMCPLogs } from "./useGetMCPLogs";
import { ScrollArea } from "../../ScrollArea";
import { Flex, Text } from "../../ui";
import { ShikiCodeBlock } from "../../Markdown/ShikiCodeBlock";
import styles from "./IntegrationForm.module.css";

type MCPLogsProps = {
  integrationPath: string;
  integrationName: string;
};

const formatMCPLogs = (logs: string[]): string => {
  return logs.join("\n");
};

export const MCPLogs: React.FC<MCPLogsProps> = ({
  integrationPath,
  integrationName,
}) => {
  const { data, isLoading } = useGetMCPLogs(integrationPath);

  if (!data) {
    if (isLoading) {
      return <Text>Loading...</Text>;
    }
    return <Text>No data</Text>;
  }

  const formattedData = formatMCPLogs(data.logs);

  return (
    <Flex className={styles.logs} direction="column" gap="4">
      <h4 className={styles.logsTitle}>
        Runtime logs of {integrationName} server
      </h4>
      <Text as="p" color="gray" size="2">
        Real-time diagnostic information from the MCP server. These logs help
        troubleshoot connection issues, monitor tool execution status, and
        verify proper server initialization. Critical for debugging when tools
        aren&apos;t appearing or functioning as expected.
      </Text>
      <ScrollArea scrollbars="horizontal" className={styles.logsScroll} asChild>
        <div className={styles.logsBox}>
          <ShikiCodeBlock
            className="language-bash"
            showLineNumbers={false}
            preOptions={{
              noMargin: true,
            }}
          >
            {formattedData}
          </ShikiCodeBlock>
        </div>
      </ScrollArea>
    </Flex>
  );
};
