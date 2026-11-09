import { Button, Flex, Text } from "@radix-ui/themes";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { selectCurrentThreadId } from "../Chat/Thread";
import { retryAppWorkspaceSave } from "./appsSlice";

export function AppWorkspaceControls() {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectCurrentThreadId);
  const status = useAppSelector((state) => state.apps.saveStatus?.[chatId]);
  if (!status) return null;
  const message =
    status === "saving"
      ? "Saving app layout…"
      : status === "saved"
        ? "App layout saved automatically."
        : status === "readonly"
          ? "This app workspace version is read-only."
          : "App layout could not be saved. Your local layout is preserved.";
  return (
    <Flex gap="2" align="center" px="2">
      <Text size="1" role="status">
        {message}
      </Text>
      {status === "error" && (
        <Button
          size="1"
          onClick={() => dispatch(retryAppWorkspaceSave(chatId))}
        >
          Retry save
        </Button>
      )}
    </Flex>
  );
}
