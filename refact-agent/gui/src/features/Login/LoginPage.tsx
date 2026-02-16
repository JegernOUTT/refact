import React, { useCallback } from "react";
import {
  Flex,
  Box,
  Button,
  Card,
  Grid,
  Text,
  Separator,
  TextField,
  Container,
  Heading,
} from "@radix-ui/themes";
import { GitHubLogoIcon } from "@radix-ui/react-icons";
import { GoogleIcon } from "../../images/GoogleIcon";
import { Accordion } from "../../components/Accordion";
import { ScrollArea } from "../../components/ScrollArea";
import {
  useAppDispatch,
  useAppSelector,
  useEmailLogin,
  useLogin,
  useGetConfiguredProvidersQuery,
} from "../../hooks";
import { ProviderCard } from "../Providers/ProviderCard";
import { ProviderPreview } from "../Providers/ProviderPreview";
import type { ProviderListItem } from "../../services/refact";
import { useGetConfiguredProvidersView } from "../Providers/ProvidersView/useConfiguredProvidersView";
import { newChatAction } from "../Chat";
import { push } from "../Pages/pagesSlice";
import { selectApiKey, selectAddressURL, setApiKey, setAddressURL } from "../Config/configSlice";

export const LoginPage: React.FC = () => {
  const { loginWithProvider, polling, cancelLogin } = useLogin();
  const { emailLogin, emailLoginResult, emailLoginAbort } = useEmailLogin();
  const dispatch = useAppDispatch();

  const apiKey = useAppSelector(selectApiKey);
  const addressURL = useAppSelector(selectAddressURL);

  const providersQuery = useGetConfiguredProvidersQuery();
  const configuredProviders = providersQuery.data?.providers ?? [];
  const { sortedConfiguredProviders } = useGetConfiguredProvidersView({
    configuredProviders,
  });
  const [currentProvider, setCurrentProvider] =
    React.useState<ProviderListItem | null>(null);

  const emailIsLoading = React.useMemo(() => {
    if (
      emailLoginResult.isSuccess &&
      emailLoginResult.data.status !== "user_logged_in"
    ) {
      return true;
    }
    return emailLoginResult.isLoading;
  }, [
    emailLoginResult.data?.status,
    emailLoginResult.isLoading,
    emailLoginResult.isSuccess,
  ]);

  const isLoading = React.useMemo(() => {
    if (polling.isLoading || polling.isFetching) return true;
    return emailIsLoading;
  }, [polling, emailIsLoading]);

  const onCancel = useCallback(() => {
    try {
      cancelLogin.current();
      emailLoginAbort();
    } catch {
      // no-op
    }
  }, [cancelLogin, emailLoginAbort]);

  const hasAnyActiveProvider = React.useMemo(() => {
    return sortedConfiguredProviders.some((p) => {
      if (p.status !== "active") return false;

      // Guard against backend versions that may mark refact configured
      // without a real Cloud login.
      if (p.name === "refact") {
        const addr = (addressURL ?? "").trim();
        const key = (apiKey ?? "").trim();
        return addr.toLowerCase() === "refact" && key.length > 0;
      }

      return true;
    });
  }, [sortedConfiguredProviders, addressURL, apiKey]);

  const onContinue = useCallback(() => {
    // BYOK path: mark as “logged in” locally without triggering SmallCloud user fetch.
    if (!addressURL || addressURL.trim().length === 0) {
      dispatch(setAddressURL("BYOK"));
    }
    if (!apiKey || apiKey.trim().length === 0) {
      dispatch(setApiKey("byok"));
    }

    dispatch(push({ name: "history" }));
    dispatch(newChatAction());
    dispatch(push({ name: "chat" }));
  }, [addressURL, apiKey, dispatch]);

  return (
    <ScrollArea scrollbars="vertical" fullHeight>
      <Container>
        <Heading align="center" as="h2" size="6" my="6">
          Login to Refact.ai
        </Heading>

        <Accordion.Root
          type="single"
          defaultValue={"cloud"}
          disabled={isLoading}
          collapsible
        >
          <Accordion.Item value="cloud">
            <Accordion.Trigger>Refact Cloud</Accordion.Trigger>
            <Accordion.Content>
            <Box>
              <Text size="2">
                <ul>
                  <li>
                    Chat with your codebase powered by top models (e.g. Claude
                    3.7 Sonnet, OpenAI GPT-4o and o3-mini).
                  </li>
                  <li>Unlimited Code Completions (powered by Qwen2.5).</li>
                  <li>Codebase-aware vector database (RAG).</li>
                  <li>
                    Agentic features: browser use, database connect, debugger,
                    shell commands, etc.
                  </li>
                </ul>
              </Text>
            </Box>
            <Separator size="4" my="4" />
            <Flex direction="column" gap="3" align="center">
              <Button
                onClick={() => {
                  onCancel();
                  loginWithProvider("google");
                }}
                disabled={isLoading}
              >
                <GoogleIcon width="15" height="15" /> Continue with Google
              </Button>
              <Button
                onClick={() => {
                  onCancel();
                  loginWithProvider("github");
                }}
                disabled={isLoading}
              >
                <GitHubLogoIcon width="15" height="15" /> Continue with GitHub
              </Button>

              <Text>or</Text>

              <Flex asChild direction="column" gap="3">
                <form
                  onSubmit={(event) => {
                    event.preventDefault();
                    if (isLoading) return;
                    const formData = new FormData(event.currentTarget);
                    const email = formData.get("email");
                    if (typeof email === "string") {
                      emailLogin(email);
                    }
                  }}
                >
                  <TextField.Root
                    placeholder="Email Address"
                    type="email"
                    name="email"
                    required
                    disabled={isLoading}
                  />
                  <Button type="submit" loading={emailIsLoading} disabled={isLoading}>
                    Send magic link
                  </Button>{" "}
                  {isLoading && <Button onClick={onCancel}>Cancel</Button>}
                  <Text size="1" align="center">
                    We will send you a one-time login link by email.
                  </Text>
                </form>
              </Flex>
            </Flex>
            </Accordion.Content>
          </Accordion.Item>
        </Accordion.Root>

        <Separator size="4" my="6" />

        {!currentProvider && (
          <>
            <Flex direction="column" gap="3">
              <Heading as="h3" size="4">
                Or bring your own provider
              </Heading>
              <Text size="2" color="gray">
                Configure one or more providers below, enable at least one
                model, then Continue.
              </Text>
            </Flex>

            <Box mt="4">
              <Grid columns={{ initial: "2", sm: "3" }} gap="3" width="100%">
                {sortedConfiguredProviders.map((provider) => (
                  <ProviderCard
                    key={provider.name}
                    provider={provider}
                    setCurrentProvider={setCurrentProvider}
                  />
                ))}
              </Grid>
            </Box>
          </>
        )}

        {currentProvider && (
          <Card mt="4" variant="surface" style={{ padding: "var(--space-4)" }}>
            <Flex justify="between" align="center" mb="3" gap="3" wrap="wrap">
              <Heading as="h4" size="3">
                {currentProvider.display_name}
              </Heading>
              <Button
                variant="outline"
                onClick={() => setCurrentProvider(null)}
              >
                Back to providers
              </Button>
            </Flex>
            <ProviderPreview
              configuredProviders={sortedConfiguredProviders}
              currentProvider={currentProvider}
              handleSetCurrentProvider={setCurrentProvider}
            />
          </Card>
        )}

        <Flex justify="end" gap="3" mt="5" align="center" wrap="wrap">
          <Text size="2" color="gray">
            {providersQuery.isFetching
              ? "Loading providers…"
              : hasAnyActiveProvider
                ? "Ready to start"
                : "Enable at least one model to continue"}
          </Text>
          <Button
            onClick={onContinue}
            disabled={
              isLoading || providersQuery.isFetching || !hasAnyActiveProvider
            }
          >
            Continue
          </Button>
        </Flex>
      </Container>
    </ScrollArea>
  );
};
