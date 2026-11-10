/* eslint-disable react-refresh/only-export-components */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createTwoFilesPatch } from "diff";
import { ArtifactBlock } from "../../components/Markdown/ArtifactBlock";
import {
  ArtifactPolicyProvider,
  type ReviewSlotPreview,
} from "../../components/Markdown/ArtifactPolicyContext";
import { Button, Dialog } from "../../components/ui";
import {
  useGetReviewFileQuery,
  type Review,
  type RichBlock,
  type TourVersion,
  type DocumentTarget,
  type ReviewFileView,
} from "../../services/refact/review";
import { BlockQuestion, RichBlockContent } from "./ReviewRichBlocks";

/** A registered reference is the only authority for a file query. Frame paths are never accepted. */
export function previewReference(
  review: Review,
  candidateId: string | undefined,
  block: RichBlock,
) {
  const c = block.content;
  const ref =
    c.kind === "code"
      ? {
          candidateId: c.anchor.position.candidate_id,
          path: c.anchor.position.path,
        }
      : c.kind === "diff"
        ? { candidateId: c.candidate_id, path: c.path }
        : null;
  return ref &&
    ref.candidateId === candidateId &&
    review.candidates.some(
      (candidate) =>
        candidate.id === candidateId &&
        candidate.files.some((f) => f.path === ref.path),
    )
    ? ref
    : null;
}

export function filePreview(block: RichBlock, file: ReviewFileView): string {
  const c = block.content;
  if (c.kind === "code") {
    const side = file[c.anchor.position.side];
    const { start_line: start, end_line: end } = c.anchor.position;
    if (side.kind !== "text")
      return "Text source unavailable (binary or absent).";
    const lines = side.text.split("\n");
    if (
      !Number.isSafeInteger(start) ||
      !Number.isSafeInteger(end) ||
      start < 1 ||
      end < start ||
      end > lines.length
    )
      return "Source range unavailable.";
    const text = lines.slice(start - 1, Math.min(end, start + 79)).join("\n");
    return end - start >= 80
      ? `${text}\n[Preview truncated: showing 80 of ${end - start + 1} lines]`
      : text;
  }
  if (file.base.kind === "binary" || file.head.kind === "binary")
    return "Binary diff: open native Review.";
  const before = file.base.kind === "text" ? file.base.text : "";
  const after = file.head.kind === "text" ? file.head.text : "";
  if (before.length + after.length > 200_000)
    return "Diff too large for preview; open native Review.";
  const options = { context: 3, timeout: 100, maxEditLength: 2000 };
  const patch = createTwoFilesPatch(
    file.path,
    file.path,
    before,
    after,
    "base",
    "head",
    options,
  );
  if (typeof patch !== "string")
    return "Diff too large for preview; open native Review.";
  const lines = patch.split("\n");
  return lines.length > 120
    ? `${lines.slice(0, 120).join("\n")}\n[Preview truncated: showing 120 of ${
        lines.length
      } patch lines]`
    : patch;
}

function PreviewData({
  review,
  candidateId,
  block,
  onData,
}: {
  review: Review;
  candidateId?: string;
  block: RichBlock;
  onData: (p: ReviewSlotPreview) => void;
}) {
  const ref = useMemo(
    () => previewReference(review, candidateId, block),
    [review, candidateId, block],
  );
  const query = useGetReviewFileQuery(
    {
      reviewId: review.id,
      candidateId: ref?.candidateId ?? "",
      path: ref?.path ?? "",
    },
    { skip: !ref },
  );
  useEffect(() => {
    const c = block.content;
    let text = "Preview unavailable.";
    let available = true;
    if (c.kind === "code" || c.kind === "diff") {
      available =
        !!ref &&
        !!query.currentData &&
        query.currentData.path === ref.path &&
        !query.isError;
      text = !ref
        ? "Referenced candidate or file is unavailable."
        : query.currentData && query.currentData.path === ref.path
          ? filePreview(block, query.currentData)
          : query.isError
            ? "Source unavailable."
            : "Loading pinned source…";
    } else if (c.kind === "sources" || c.kind === "evidence") {
      text = c.evidence_ids
        .map((id) => {
          const item = review.evidence.find(
            (e) => e.id === id && e.candidate_id === candidateId,
          );
          if (!item) return `Evidence ${id}: unavailable for this candidate`;
          const s = item.source;
          return `${item.summary}\n${
            s.kind === "source"
              ? s.path
              : s.kind === "external"
                ? s.url
                : `${s.command.join(" ")} · exit ${s.exit_code ?? "unknown"}`
          }\nEvidence ${item.id}`;
        })
        .join("\n\n");
    } else {
      available = false;
      text = "This block has no lightweight preview.";
    }
    if (text.length > 12000)
      text = `${text.slice(
        0,
        12000,
      )}\n[Preview truncated: showing 12000 characters; open native Review for full content]`;
    onData({
      id: block.id,
      title: (ref?.path ?? c.kind).slice(0, 512),
      text,
      available,
    });
  }, [
    review,
    candidateId,
    block,
    ref,
    query.currentData,
    query.isError,
    onData,
  ]);
  return null;
}

function HtmlPageInstance({
  review,
  version,
  stepId,
  target,
  revealTarget,
}: {
  revealTarget?: { target: DocumentTarget; token: number; threadId?: string };
  review: Review;
  version: TourVersion;
  stepId: string;
  target?: DocumentTarget;
}) {
  const [data, setData] = useState<Partial<Record<string, ReviewSlotPreview>>>(
    {},
  );
  const [opened, setOpened] = useState<{
    id: string;
    action: "reveal" | "comment";
  } | null>(null);
  const blocks = useMemo(
    () =>
      (version.blocks ?? [])
        .filter(
          (b, _, all) => all.filter((other) => other.id === b.id).length === 1,
        )
        .slice(0, 32),
    [version.blocks],
  );
  const onData = useMemo(
    () => (p: ReviewSlotPreview) =>
      setData((old) =>
        JSON.stringify(old[p.id]) === JSON.stringify(p)
          ? old
          : { ...old, [p.id]: p },
      ),
    [],
  );
  const previews = blocks.map(
    (block) =>
      data[block.id] ?? {
        id: block.id,
        title: block.content.kind,
        text: "Loading preview…",
        available: false,
      },
  );
  const consumedReveal = useRef<number | null>(null);
  const openedReveal = useRef<number | null>(null);
  useEffect(() => {
    if (
      !revealTarget ||
      openedReveal.current === revealTarget.token ||
      revealTarget.target.step_id !== stepId ||
      revealTarget.target.version !== version.version ||
      revealTarget.target.candidate_id !== target?.candidate_id
    )
      return;
    const block = blocks.find(
      (b) =>
        b.id === revealTarget.target.block_id &&
        (!revealTarget.target.block_revision ||
          b.revision === revealTarget.target.block_revision),
    );
    if (block) {
      openedReveal.current = revealTarget.token;
      setOpened({ id: block.id, action: "comment" });
    }
  }, [revealTarget, stepId, version.version, target?.candidate_id, blocks]);
  const selected = blocks.find((block) => block.id === opened?.id);
  const threadAvailable =
    !revealTarget?.threadId ||
    review.threads.some(
      (thread) =>
        thread.id === revealTarget.threadId &&
        thread.target?.block_id === selected?.id &&
        thread.target?.step_id === stepId &&
        thread.target.version === version.version &&
        thread.target.candidate_id === target?.candidate_id,
    );
  const focusThread = useCallback(
    (node: HTMLDivElement | null) => {
      if (
        !node ||
        !revealTarget?.threadId ||
        !threadAvailable ||
        openedReveal.current !== revealTarget.token ||
        consumedReveal.current === revealTarget.token
      )
        return;
      const token = revealTarget.token;
      requestAnimationFrame(() => {
        if (!node.isConnected || consumedReveal.current === token) return;
        const thread = Array.from(
          node.querySelectorAll<HTMLElement>("[data-document-thread-id]"),
        ).find((el) => el.dataset.documentThreadId === revealTarget.threadId);
        if (!thread) return;
        consumedReveal.current = token;
        thread.tabIndex = -1;
        thread.scrollIntoView({ block: "nearest" });
        thread.focus({ preventScroll: true });
      });
    },
    [revealTarget, threadAvailable],
  );

  return (
    <>
      {(version.blocks?.length ?? 0) > 32 && (
        <p role="status">
          Preview limit: showing at most 32 of {version.blocks?.length}{" "}
          registered blocks.
        </p>
      )}
      {blocks.map((block) => (
        <PreviewData
          key={block.id}
          review={review}
          candidateId={target?.candidate_id}
          block={block}
          onData={onData}
        />
      ))}
      <ArtifactPolicyProvider
        value={{
          mode: "review-seamless",
          title: version.title,
          bindingKey: `${review.id}:${stepId}:${version.version}`,
          page: {
            previews,
            onAction: (action, id) => {
              if (previews.some((p) => p.id === id && p.available))
                setOpened({ id, action });
            },
          },
        }}
      >
        <ArtifactBlock
          code={version.html ?? ""}
          staticFallback={
            version.body.slice(0, 600) ||
            "Open the live page to explore this topic."
          }
        />
      </ArtifactPolicyProvider>
      <Dialog
        open={!!selected}
        onOpenChange={(open) => {
          if (!open) setOpened(null);
        }}
      >
        <Dialog.Content maxWidth="1100px">
          <div ref={focusThread}>
            <Dialog.Title>
              {opened?.action === "comment"
                ? "Comment on block"
                : "Native Review block"}
            </Dialog.Title>
            <Dialog.Description>
              Candidate-pinned source and discussion · {selected?.id}
            </Dialog.Description>
            {selected && opened?.action === "reveal" && (
              <RichBlockContent
                review={review}
                block={selected}
                bindingKey={`${review.id}:${stepId}:${selected.id}`}
              />
            )}
            {selected && target && (
              <BlockQuestion
                key={`${selected.id}:${opened?.action}`}
                review={review}
                target={{
                  ...target,
                  block_id: selected.id,
                  block_revision: selected.revision,
                }}
                initiallyOpen={opened?.action === "comment"}
              />
            )}
            {!threadAvailable && (
              <p role="status">
                Requested discussion is unavailable for this block version.
              </p>
            )}
          </div>
          <Dialog.Footer>
            <Dialog.Close asChild>
              <Button>Close</Button>
            </Dialog.Close>
          </Dialog.Footer>
        </Dialog.Content>
      </Dialog>
    </>
  );
}

export function ReviewHtmlPage(props: {
  revealTarget?: { target: DocumentTarget; token: number; threadId?: string };
  review: Review;
  version: TourVersion;
  stepId: string;
  target?: DocumentTarget;
}) {
  const identity = JSON.stringify([
    props.review.id,
    props.stepId,
    props.target?.candidate_id,
    props.version.version,
    props.version.blocks,
  ]);
  return <HtmlPageInstance key={identity} {...props} />;
}
