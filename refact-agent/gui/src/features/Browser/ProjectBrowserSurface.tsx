import { Button, IconButton, TextField } from "@radix-ui/themes";
import { ArrowLeft, ArrowRight, Globe, Plus, RotateCw, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { ClipboardEvent, KeyboardEvent, MouseEvent } from "react";
import {
  browserApi,
  projectBrowserErrorMessage,
  type ProjectBrowserFrame,
  type ProjectBrowserInputFence,
  type ProjectBrowserInputRequest,
  type ProjectBrowserModifier,
} from "../../services/refact/browser";
import { framePosition } from "./projectBrowserFrame";
import { useProjectBrowser } from "./useProjectBrowser";
import styles from "./ProjectBrowserSurface.module.css";

function MigrationChooser({
  busy,
  resolve,
}: {
  busy: boolean;
  resolve: (legacyChatId?: string) => void;
}) {
  const [getCandidates, { data, error }] =
    browserApi.useProjectBrowserMigrationCandidatesMutation();
  const [candidate, setCandidate] = useState("");
  useEffect(() => {
    void getCandidates(undefined);
  }, [getCandidates]);
  const selected = data?.candidates.find(
    (item) => item.legacy_chat_id === candidate,
  );

  return (
    <div className={styles.message}>
      <h2>Choose a browser profile</h2>
      <p>
        Move one chat’s browser profile into the project browser, or start
        fresh. Other chat profiles will not be changed.
      </p>
      {error ? (
        <>
          <p role="alert">{projectBrowserErrorMessage(error)}</p>
          <Button variant="soft" onClick={() => void getCandidates(undefined)}>
            Retry loading profiles
          </Button>
        </>
      ) : data ? (
        <>
          {data.candidates.length > 0 && (
            <label className={styles.choose}>
              Chat browser profile
              <select
                value={candidate}
                onChange={(event) => setCandidate(event.currentTarget.value)}
                disabled={busy}
              >
                <option value="">Select a profile</option>
                {data.candidates.map((profile) => (
                  <option
                    key={profile.legacy_chat_id}
                    value={profile.legacy_chat_id}
                    disabled={profile.active}
                  >
                    {profile.legacy_chat_id}
                    {profile.active ? " (in use)" : ""}
                  </option>
                ))}
              </select>
            </label>
          )}
          <div className={styles.actions}>
            <Button
              disabled={busy || !selected || selected.active}
              onClick={() => resolve(candidate)}
            >
              Adopt profile
            </Button>
            <Button variant="soft" disabled={busy} onClick={() => resolve()}>
              Start fresh
            </Button>
          </div>
        </>
      ) : (
        <p role="status">Loading browser profiles…</p>
      )}
    </div>
  );
}

function BrowserFrame({
  frame,
  disabled,
  onInput,
}: {
  frame: ProjectBrowserFrame;
  disabled: boolean;
  onInput: (request: ProjectBrowserInputRequest) => void;
}) {
  const image = useRef<HTMLImageElement>(null);

  useEffect(() => {
    const element = image.current;
    const wheel = (event: WheelEvent) => {
      if (!element || disabled || event.ctrlKey || event.metaKey) return;
      const bounds = element.getBoundingClientRect();
      const point = framePosition(frame, bounds, event.clientX, event.clientY);
      if (!point) return;
      event.preventDefault();
      const scale =
        event.deltaMode === 1
          ? 16
          : event.deltaMode === 2
            ? frame.viewport.height
            : 1;
      const delta = (value: number) =>
        Math.min(10000, Math.max(-10000, value * scale));
      onInput({
        runtime_id: frame.runtime_id,
        tab_id: frame.tab_id,
        kind: "wheel",
        ...point,
        dx: delta(event.deltaX),
        dy: delta(event.deltaY),
      });
    };
    element?.addEventListener("wheel", wheel, { passive: false });
    return () => element?.removeEventListener("wheel", wheel);
  }, [disabled, frame, onInput]);

  const fence: ProjectBrowserInputFence = {
    runtime_id: frame.runtime_id,
    tab_id: frame.tab_id,
  };
  const click = (event: MouseEvent<HTMLImageElement>) => {
    if (disabled) return;
    const point = framePosition(
      frame,
      event.currentTarget.getBoundingClientRect(),
      event.clientX,
      event.clientY,
    );
    if (!point || event.button > 2) return;
    event.preventDefault();
    event.currentTarget.focus();
    const button =
      event.button === 0 ? "left" : event.button === 1 ? "middle" : "right";
    onInput({ ...fence, kind: "click", button, ...point });
  };

  const keyDown = (event: KeyboardEvent<HTMLImageElement>) => {
    if (
      disabled ||
      event.nativeEvent.isComposing ||
      event.key === "Tab" ||
      event.key === "Escape"
    )
      return;
    if (
      ["Control", "Meta", "Shift", "Alt", "Unidentified", "Dead"].includes(
        event.key,
      )
    )
      return;
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "v")
      return;
    event.preventDefault();
    event.stopPropagation();
    const modifiers: ProjectBrowserModifier[] = [];
    if (event.altKey) modifiers.push("Alt");
    if (event.ctrlKey) modifiers.push("Ctrl");
    if (event.metaKey) modifiers.push("Meta");
    if (event.shiftKey) modifiers.push("Shift");
    if (
      Array.from(event.key).length === 1 &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey
    ) {
      onInput({ ...fence, kind: "text", text: event.key });
    } else {
      onInput({ ...fence, kind: "key", key: event.key, modifiers });
    }
  };

  const paste = (event: ClipboardEvent<HTMLImageElement>) => {
    if (disabled) return;
    event.preventDefault();
    event.stopPropagation();
    const text = Array.from(event.clipboardData.getData("text/plain"))
      .slice(0, 8192)
      .join("");
    if (text) onInput({ ...fence, kind: "text", text });
  };

  return (
    <img
      ref={image}
      className={styles.frame}
      src={`data:${frame.mime};base64,${frame.data}`}
      alt="Browser page"
      tabIndex={disabled ? -1 : 0}
      draggable={false}
      onClick={click}
      onAuxClick={click}
      onContextMenu={(event) => event.preventDefault()}
      onKeyDown={keyDown}
      onPaste={paste}
    />
  );
}

export function ProjectBrowserSurface() {
  const { busy, command, content, error, frame, sendInput, start, status } =
    useProjectBrowser();
  const [changeTab] = browserApi.useProjectBrowserTabMutation();
  const [navigate] = browserApi.useProjectBrowserNavigateMutation();
  const [adopt] = browserApi.useProjectBrowserMigrationAdoptMutation();
  const [skip] = browserApi.useProjectBrowserMigrationSkipMutation();
  const [address, setAddress] = useState<string>();
  const activeTab = status?.tabs.find(
    (tab) => tab.tab_id === status.active_tab,
  );
  const hasBrowser = status?.connected === true;
  const controlsDisabled = busy || status?.busy === true;
  const activeUrl = frame?.url ?? activeTab?.url ?? "";

  const resolve = (legacyChatId?: string) =>
    void command(async () => {
      if (legacyChatId) await adopt({ legacy_chat_id: legacyChatId }).unwrap();
      else await skip(undefined).unwrap();
      return start(undefined).unwrap();
    });

  return (
    <section className={styles.surface} aria-label="Project browser">
      {hasBrowser && (
        <>
          <div className={styles.tabs} role="group" aria-label="Browser tabs">
            {status.tabs.map((tab) => {
              const label =
                tab.title ||
                (tab.url !== "about:blank" ? tab.url : "New tab") ||
                "New tab";
              const selected = tab.tab_id === status.active_tab;
              return (
                <div className={styles.tab} key={tab.tab_id}>
                  <Button
                    size="1"
                    variant={selected ? "soft" : "ghost"}
                    className={styles.tabTitle}
                    title={label}
                    aria-pressed={selected}
                    disabled={controlsDisabled}
                    onClick={() => {
                      setAddress(undefined);
                      void command(() =>
                        changeTab({
                          op: "select",
                          tab_id: tab.tab_id,
                        }).unwrap(),
                      );
                    }}
                  >
                    {label}
                  </Button>
                  <IconButton
                    type="button"
                    size="1"
                    variant="ghost"
                    aria-label={`Close tab: ${label}`}
                    disabled={controlsDisabled}
                    onClick={() => {
                      setAddress(undefined);
                      void command(() =>
                        changeTab({ op: "close", tab_id: tab.tab_id }).unwrap(),
                      );
                    }}
                  >
                    <X size={14} />
                  </IconButton>
                </div>
              );
            })}
            <IconButton
              type="button"
              size="1"
              variant="ghost"
              aria-label="New tab"
              title="New tab"
              disabled={controlsDisabled}
              onClick={() => {
                setAddress(undefined);
                void command(() => changeTab({ op: "new" }).unwrap());
              }}
            >
              <Plus size={16} />
            </IconButton>
          </div>
          <form
            className={styles.navigation}
            aria-label="Browser navigation"
            onSubmit={(event) => {
              event.preventDefault();
              const url = (address ?? activeUrl).trim();
              if (!activeTab || !url || controlsDisabled) return;
              setAddress(undefined);
              void command(() =>
                navigate({ url, tab_id: activeTab.tab_id }).unwrap(),
              );
            }}
          >
            {(
              [
                ["back", "Back", ArrowLeft],
                ["forward", "Forward", ArrowRight],
                ["reload", "Reload", RotateCw],
              ] as const
            ).map(([op, label, Icon]) => (
              <IconButton
                key={op}
                type="button"
                size="1"
                variant="ghost"
                aria-label={label}
                title={label}
                disabled={controlsDisabled || !activeTab}
                onClick={() => {
                  if (!activeTab) return;
                  setAddress(undefined);
                  void command(() =>
                    navigate({ op, tab_id: activeTab.tab_id }).unwrap(),
                  );
                }}
              >
                <Icon size={16} />
              </IconButton>
            ))}
            <TextField.Root
              className={styles.address}
              size="1"
              aria-label="Address"
              placeholder="Enter a URL"
              value={address ?? activeUrl}
              disabled={controlsDisabled || !activeTab}
              onChange={(event) => setAddress(event.currentTarget.value)}
              onKeyDown={(event) => {
                if (event.key === "Escape") setAddress(undefined);
              }}
            />
            <Button
              type="submit"
              size="1"
              variant="soft"
              disabled={controlsDisabled || !activeTab}
            >
              Go
            </Button>
          </form>
        </>
      )}
      {error && (
        <div className={styles.error} role="alert">
          {error}
        </div>
      )}
      <div className={styles.content} ref={content}>
        {status?.migration.state === "available" && !hasBrowser ? (
          <MigrationChooser busy={controlsDisabled} resolve={resolve} />
        ) : frame ? (
          <BrowserFrame
            frame={frame}
            disabled={controlsDisabled}
            onInput={sendInput}
          />
        ) : (
          <div className={styles.message}>
            <Globe size={24} aria-hidden />
            {!status ? (
              <p role="status">Checking project browser…</p>
            ) : busy ? (
              <p role="status">
                {hasBrowser ? "Updating browser…" : "Starting browser…"}
              </p>
            ) : hasBrowser ? (
              <p role="status">
                {activeTab
                  ? "Waiting for a browser frame…"
                  : "Open a tab to browse."}
              </p>
            ) : (
              <Button
                onClick={() => void command(() => start(undefined).unwrap())}
              >
                Start browser
              </Button>
            )}
          </div>
        )}
      </div>
    </section>
  );
}
