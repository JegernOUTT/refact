import { Box, Button, Flex, Heading } from "../LongTailPrimitives";
import { ScrollArea } from "../ScrollArea";
import { ShikiCodeBlock } from "../Markdown/ShikiCodeBlock";

type ChatRawJSONProps = {
  thread: { title?: string; [key: string]: unknown };
  copyHandler: () => void;
};

export const ChatRawJSON = ({ thread, copyHandler }: ChatRawJSONProps) => {
  return (
    <Box
      style={{
        width: "100%",
        height: "100%",
        maxHeight: "92%",
        flexGrow: 1,
      }}
    >
      <Flex
        direction="column"
        align={"start"}
        style={{
          width: "100%",
          maxWidth: "100%",
          height: "100%",
          maxHeight: "97%",
        }}
      >
        <Heading as="h3" align="center" mb="2">
          Thread History
        </Heading>
        {thread.title && (
          <Heading as="h6" size="2" align="center" mb="4">
            {thread.title}
          </Heading>
        )}
        <Flex
          align="start"
          justify="center"
          direction="column"
          width="100%"
          maxHeight="75%"
        >
          <ScrollArea scrollbars="horizontal" style={{ width: "100%" }} asChild>
            <Box>
              <ShikiCodeBlock
                className="language-json"
                preOptions={{ noMargin: true }}
              >
                {JSON.stringify(thread, null, 2)}
              </ShikiCodeBlock>
            </Box>
          </ScrollArea>
        </Flex>
        <Flex mt="5" gap="3" align="center" justify="center">
          <Button onClick={copyHandler}>Copy to clipboard</Button>
        </Flex>
      </Flex>
    </Box>
  );
};
