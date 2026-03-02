import React, { useState, useCallback, useEffect } from "react";
import {
  Badge,
  Box,
  Button,
  Card,
  Flex,
  Select,
  Spinner,
  Text,
  TextField,
} from "@radix-ui/themes";
import { useDebounceCallback } from "usehooks-ts";
import {
  useSearchSkillsQuery,
  useAiSearchSkillsQuery,
} from "../../../services/refact/skillsmp";
import type { SkillEntry, SkillsRateLimit } from "../../../services/refact/skillsmp";

import styles from "./SkillsMPPanel.module.css";

const STORAGE_KEY = "skillsmp_api_key";

type SearchMode = "keyword" | "ai";
type SortBy = "stars" | "recent";

const SkillCard: React.FC<{ skill: SkillEntry }> = ({ skill }) => {
  const handleOpen = useCallback(() => {
    if (skill.repo) {
      window.open(skill.repo, "_blank", "noopener,noreferrer");
    }
  }, [skill.repo]);

  return (
    <Card className={styles.skillCard}>
      <Flex direction="column" gap="2" height="100%">
        <Flex justify="between" align="start" gap="2">
          <Box style={{ flex: 1, minWidth: 0 }}>
            <Text size="2" weight="bold">
              {skill.name}
            </Text>
            {skill.description && (
              <Text size="1" as="p" className={styles.description} mt="1">
                {skill.description}
              </Text>
            )}
          </Box>
          {skill.stars !== undefined && (
            <Badge size="1" color="yellow" variant="soft">
              ⭐ {skill.stars}
            </Badge>
          )}
        </Flex>

        {skill.author && (
          <Flex gap="1" wrap="wrap">
            <Badge size="1" color="gray" variant="soft">
              {skill.author}
            </Badge>
          </Flex>
        )}

        {skill.repo && (
          <Button size="1" variant="soft" onClick={handleOpen} style={{ marginTop: "auto" }}>
            Open on GitHub
          </Button>
        )}
      </Flex>
    </Card>
  );
};

const RateLimitInfo: React.FC<{ ratelimit: SkillsRateLimit }> = ({ ratelimit }) => {
  const low = ratelimit.daily_remaining < 50;
  return (
    <Text size="1" color={low ? "orange" : "gray"} className={styles.rateLimit}>
      {ratelimit.daily_remaining} / {ratelimit.daily_limit} requests remaining today
    </Text>
  );
};

type ResultsProps = {
  q: string;
  mode: SearchMode;
  sortBy: SortBy;
  apiKey: string;
  onRateLimit: (rl: SkillsRateLimit) => void;
};

const Results: React.FC<ResultsProps> = ({ q, mode, sortBy, apiKey, onRateLimit }) => {
  const skip = !q || !apiKey;

  const keywordResult = useSearchSkillsQuery(
    { q, sort_by: sortBy, apiKey },
    { skip: skip || mode !== "keyword" },
  );

  const aiResult = useAiSearchSkillsQuery(
    { q, apiKey },
    { skip: skip || mode !== "ai" },
  );

  const result = mode === "keyword" ? keywordResult : aiResult;

  useEffect(() => {
    if (result.data?.ratelimit) {
      onRateLimit(result.data.ratelimit);
    }
  }, [result.data, onRateLimit]);

  if (!apiKey) {
    return (
      <Text size="2" color="gray" className={styles.emptyState}>
        Enter your API key above to search SkillsMP.
      </Text>
    );
  }

  if (!q) {
    return (
      <Text size="2" color="gray" className={styles.emptyState}>
        Enter a search query to find skills.
      </Text>
    );
  }

  if (result.isLoading) {
    return (
      <Flex align="center" justify="center" gap="2" py="6">
        <Spinner size="2" />
        <Text size="2" color="gray">
          Searching…
        </Text>
      </Flex>
    );
  }

  if (result.isError) {
    const errStatus =
      "status" in result.error && typeof result.error.status === "number"
        ? result.error.status
        : undefined;

    if (errStatus === 401) {
      return (
        <Text size="2" color="red" className={styles.emptyState}>
          Invalid API key. Please check your key.
        </Text>
      );
    }

    if (errStatus === 429) {
      return (
        <Text size="2" color="orange" className={styles.emptyState}>
          Daily quota exceeded. Try again tomorrow.
        </Text>
      );
    }

    return (
      <Flex direction="column" align="center" gap="2" py="6">
        <Text size="2" color="red">
          An error occurred. Please try again.
        </Text>
        <Button size="1" variant="soft" onClick={() => void result.refetch()}>
          Retry
        </Button>
      </Flex>
    );
  }

  const skills = result.data?.data.skills ?? [];

  if (skills.length === 0) {
    return (
      <Text size="2" color="gray" className={styles.emptyState}>
        No skills found. Try a different search.
      </Text>
    );
  }

  return (
    <div className={styles.skillsGrid}>
      {skills.map((skill, idx) => (
        <SkillCard key={`${skill.name}-${idx}`} skill={skill} />
      ))}
    </div>
  );
};

export const SkillsMPPanel: React.FC = () => {
  const [apiKey, setApiKey] = useState<string>(() => localStorage.getItem(STORAGE_KEY) ?? "");
  const [keyInput, setKeyInput] = useState("");
  const [showKeyForm, setShowKeyForm] = useState(() => !localStorage.getItem(STORAGE_KEY));
  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [mode, setMode] = useState<SearchMode>("keyword");
  const [sortBy, setSortBy] = useState<SortBy>("stars");
  const [rateLimit, setRateLimit] = useState<SkillsRateLimit | undefined>(undefined);

  const debouncedSetSearch = useDebounceCallback(setDebouncedSearch, 500);

  const handleSearchChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      setSearch(e.target.value);
      debouncedSetSearch(e.target.value);
    },
    [debouncedSetSearch],
  );

  const handleSaveKey = useCallback(() => {
    const trimmed = keyInput.trim();
    if (!trimmed) return;
    localStorage.setItem(STORAGE_KEY, trimmed);
    setApiKey(trimmed);
    setShowKeyForm(false);
    setKeyInput("");
  }, [keyInput]);

  const handleChangeKey = useCallback(() => {
    setShowKeyForm(true);
  }, []);

  return (
    <div className={styles.panel}>
      {showKeyForm ? (
        <Box className={styles.keySection}>
          <Text size="2" weight="bold" mb="2" as="p">
            SkillsMP API Key
          </Text>
          <Text size="1" color="gray" mb="3" as="p">
            Get your free API key at{" "}
            <a href="https://skillsmp.com/docs/api" target="_blank" rel="noopener noreferrer">
              skillsmp.com/docs/api
            </a>
          </Text>
          <Flex gap="2" align="center">
            <Box style={{ flex: 1 }}>
              <TextField.Root
                type="password"
                placeholder="sk_live_skillsmp_..."
                value={keyInput}
                onChange={(e) => setKeyInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleSaveKey();
                }}
              />
            </Box>
            <Button onClick={handleSaveKey} disabled={!keyInput.trim()}>
              Save
            </Button>
          </Flex>
        </Box>
      ) : (
        <Flex justify="end">
          <Button size="1" variant="ghost" color="gray" onClick={handleChangeKey}>
            Change API key
          </Button>
        </Flex>
      )}

      <div className={styles.searchRow}>
        <Box className={styles.searchInput}>
          <TextField.Root
            placeholder="Search 280k+ agent skills..."
            value={search}
            onChange={handleSearchChange}
          />
        </Box>
        <Select.Root
          value={mode}
          onValueChange={(v) => setMode(v as SearchMode)}
        >
          <Select.Trigger />
          <Select.Content>
            <Select.Item value="keyword">Keyword</Select.Item>
            <Select.Item value="ai">AI Search</Select.Item>
          </Select.Content>
        </Select.Root>
        {mode === "keyword" && (
          <Select.Root
            value={sortBy}
            onValueChange={(v) => setSortBy(v as SortBy)}
          >
            <Select.Trigger />
            <Select.Content>
              <Select.Item value="stars">By stars</Select.Item>
              <Select.Item value="recent">By recent</Select.Item>
            </Select.Content>
          </Select.Root>
        )}
      </div>

      {rateLimit && <RateLimitInfo ratelimit={rateLimit} />}

      <Results
        q={debouncedSearch}
        mode={mode}
        sortBy={sortBy}
        apiKey={apiKey}
        onRateLimit={setRateLimit}
      />
    </div>
  );
};
