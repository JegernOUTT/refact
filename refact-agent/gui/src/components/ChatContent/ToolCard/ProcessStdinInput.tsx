import React, { useCallback, useState } from "react";
import { Flex, Text } from "@radix-ui/themes";
import { Button, Field, FieldText } from "../../ui";

import { useAppSelector } from "../../../hooks";
import {
  selectApiKey,
  selectConfig,
} from "../../../features/Config/configSlice";
import { hasUsableEngineEndpoint } from "../../../services/refact/apiUrl";
import { writeProcessStdin } from "../../../services/refact/exec";
import styles from "./ExecToolCard.module.css";

type ProcessStdinInputProps = {
  processId: string;
};

export const ProcessStdinInput: React.FC<ProcessStdinInputProps> = ({
  processId,
}) => {
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);
  const [chars, setChars] = useState("");
  const [isSending, setIsSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const hasEngineEndpoint = hasUsableEngineEndpoint(config);

  const sendChars = useCallback(
    async (value: string) => {
      if (!hasEngineEndpoint || isSending || value.length === 0) return;
      setIsSending(true);
      setError(null);
      try {
        await writeProcessStdin(processId, value, config, apiKey ?? undefined);
        setChars("");
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : String(cause));
      } finally {
        setIsSending(false);
      }
    },
    [apiKey, config, hasEngineEndpoint, isSending, processId],
  );

  const canSend = chars.length > 0 && !isSending && hasEngineEndpoint;

  return (
    <Flex direction="column" gap="2" className={styles.stdinInputRow}>
      <Text size="1" color="gray" className={styles.stdinBanner}>
        Interactive process — direct stdin available
      </Text>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          event.stopPropagation();
          void sendChars(chars);
        }}
      >
        <Flex gap="2" align="center">
          <Field className={styles.stdinTextField}>
            <FieldText
              aria-label="Process stdin"
              value={chars}
              placeholder="Type stdin..."
              disabled={isSending || !hasEngineEndpoint}
              onChange={setChars}
              onClick={(event) => event.stopPropagation()}
            />
          </Field>
          <Button
            type="submit"
            size="sm"
            disabled={!canSend}
            onClick={(event) => event.stopPropagation()}
          >
            Send
          </Button>
          <Button
            type="button"
            size="sm"
            variant="soft"
            disabled={isSending || !hasEngineEndpoint}
            onClick={(event) => {
              event.stopPropagation();
              void sendChars("\u0003");
            }}
          >
            Send Ctrl+C
          </Button>
        </Flex>
      </form>
      {error && (
        <Text size="1" color="red">
          {error}
        </Text>
      )}
    </Flex>
  );
};

export default ProcessStdinInput;
