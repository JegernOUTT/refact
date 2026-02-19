import { ArrowDownIcon } from "@radix-ui/react-icons";
import { Container, Flex, IconButton } from "@radix-ui/themes";

type ScrollToBottomButtonProps = {
  onClick: () => void;
};

export const ScrollToBottomButton = ({
  onClick,
}: ScrollToBottomButtonProps) => {
  return (
    <Container
      style={{
        position: "absolute",
        bottom: 15,
        left: 0,
        right: 0,
        pointerEvents: "none",
      }}
    >
      <Flex justify="end" pr="4">
        <IconButton
          title="Follow stream"
          style={{
            width: 35,
            height: 35,
            zIndex: 1,
            pointerEvents: "auto",
          }}
          onClick={onClick}
        >
          <ArrowDownIcon width={21} height={21} />
        </IconButton>
      </Flex>
    </Container>
  );
};
