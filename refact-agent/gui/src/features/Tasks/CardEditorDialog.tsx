import { useEffect, useId, useRef, useState } from "react";
import {
  Button,
  Dialog,
  Field,
  FieldSelect,
  FieldText,
  FieldTextarea,
} from "../../components/ui";
import { Checkbox } from "../../components/Checkbox";
import {
  taskErrorMessage,
  usePatchBoardMutation,
  type BoardPatch,
  type TaskBoard,
} from "../../services/refact/tasks";
import { humanizeIdentifier } from "../../utils/displayNames";
import {
  cardDraftError,
  initialCardDraft,
  type CardEditTarget,
} from "./cardForm";
import styles from "./CardManagement.module.css";

interface CardEditorDialogProps {
  taskId: string;
  board: TaskBoard;
  target: CardEditTarget;
  boardFetching: boolean;
  boardUnavailable: boolean;
  onReload: () => Promise<TaskBoard>;
  onClose: () => void;
}

const MOVE_COLUMNS = ["planned", "doing", "done", "failed", "regressed"];

export function CardEditorDialog({
  taskId,
  board,
  target,
  boardFetching,
  boardUnavailable,
  onReload,
  onClose,
}: CardEditorDialogProps) {
  const fieldId = useId();
  const [draft, setDraft] = useState(() => initialCardDraft(target));
  const [column, setColumn] = useState(
    target.mode === "create" ? "planned" : target.card.column,
  );
  const [reviewedRev, setReviewedRev] = useState(board.rev);
  const [conflict, setConflict] = useState<{ minimumRev: number } | null>(null);
  const [reloadedRev, setReloadedRev] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reloading, setReloading] = useState(false);
  const [patchBoard, { isLoading }] = usePatchBoardMutation();
  const inFlight = useRef(false);
  const reloadInFlight = useRef(false);
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const creating = target.mode === "create";
  const editing = creating || target.mode === "edit";
  const cardId = creating ? draft.id.trim() : draft.id;
  const currentCard = board.cards.find((card) => card.id === cardId);
  const needsReview = conflict !== null || board.rev !== reviewedRev;
  const canReview =
    !boardFetching &&
    !boardUnavailable &&
    !reloading &&
    (!conflict ||
      (reloadedRev !== null &&
        reloadedRev >= conflict.minimumRev &&
        board.rev >= conflict.minimumRev));
  const pending = isLoading || reloading;
  const title = creating
    ? "Create card"
    : target.mode === "edit"
      ? "Edit card"
      : target.mode === "move"
        ? "Move card"
        : "Delete card";

  const reload = async () => {
    if (reloadInFlight.current) return;
    reloadInFlight.current = true;
    setReloading(true);
    setReloadedRev(null);
    try {
      const latest = await onReload();
      if (mounted.current) setReloadedRev(latest.rev);
    } catch (err) {
      if (mounted.current)
        setError(`Could not reload board: ${taskErrorMessage(err)}`);
    } finally {
      reloadInFlight.current = false;
      if (mounted.current) setReloading(false);
    }
  };

  const submit = async () => {
    if (
      inFlight.current ||
      pending ||
      needsReview ||
      boardUnavailable ||
      boardFetching
    )
      return;
    let patch: BoardPatch;
    if (editing) {
      const validationError = cardDraftError(draft, board, creating);
      if (validationError) {
        setError(validationError);
        return;
      }
      patch = {
        action: creating ? "create_card" : "update_card",
        id: cardId,
        title: draft.title.trim(),
        priority: draft.priority,
        instructions: draft.instructions,
        target_files: draft.targetFiles
          .split(/\r?\n/)
          .map((path) => path.trim())
          .filter(Boolean),
        depends_on: [...draft.dependsOn],
      };
    } else if (target.mode === "move") {
      if (!MOVE_COLUMNS.includes(column) || column === currentCard?.column) {
        setError("Choose a different supported column.");
        return;
      }
      patch = { action: "move_card", id: draft.id, column };
    } else {
      patch = { action: "delete_card", card_id: draft.id, force: false };
    }
    inFlight.current = true;
    setError(null);
    try {
      await patchBoard({
        taskId,
        patch: { rev: reviewedRev, ...patch },
      }).unwrap();
      if (mounted.current) onClose();
    } catch (err) {
      if (!mounted.current) return;
      const message = taskErrorMessage(err);
      setError(`${title} failed: ${message}`);
      if (
        typeof err === "object" &&
        err &&
        "status" in err &&
        err.status === 409
      ) {
        const revision = /Board rev mismatch: expected \d+, actual (\d+)/.exec(
          message,
        );
        setConflict({
          minimumRev: revision ? Number(revision[1]) : reviewedRev,
        });
        await reload();
      }
    } finally {
      inFlight.current = false;
    }
  };

  const dependencyChoices = board.cards.filter((card) => card.id !== cardId);
  const unknownDependencies = draft.dependsOn.filter(
    (id) => !dependencyChoices.some((card) => card.id === id),
  );

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !inFlight.current && !reloadInFlight.current) onClose();
      }}
    >
      <Dialog.Content
        maxWidth="640px"
        maxHeight="calc(100dvh - var(--rf-space-5))"
      >
        <Dialog.Title>{title}</Dialog.Title>
        <Dialog.Description>
          {creating
            ? "New cards start in Planned with P1 priority. Creating a card does not start an agent."
            : `${draft.id}: ${target.card.title}`}
        </Dialog.Description>
        <div className={styles.editorBody}>
          {error && (
            <div role="alert" className={styles.error}>
              {error}
            </div>
          )}
          {needsReview && (
            <section
              className={styles.review}
              aria-label="Board revision review"
            >
              <div role="alert">
                The board changed or refused this request. Your draft is kept.
                Review the current board, then explicitly confirm its revision
                before submitting again. Nothing is retried automatically.
              </div>
              <div>
                Draft revision {reviewedRev}; current board revision {board.rev}
                .
              </div>
              {currentCard ? (
                <dl className={styles.currentValues}>
                  <dt>Current title</dt>
                  <dd>{currentCard.title}</dd>
                  <dt>Current priority</dt>
                  <dd>{currentCard.priority}</dd>
                  <dt>Current column</dt>
                  <dd>{humanizeIdentifier(currentCard.column)}</dd>
                  <dt>Current dependencies</dt>
                  <dd>{currentCard.depends_on.join(", ") || "None"}</dd>
                  <dt>Current instructions</dt>
                  <dd>{currentCard.instructions || "None"}</dd>
                  <dt>Current target files</dt>
                  <dd>{currentCard.target_files.join("\n") || "None"}</dd>
                </dl>
              ) : (
                <div>
                  {creating
                    ? "This ID is not on the current board."
                    : "This card is no longer on the current board. Your draft is kept."}
                </div>
              )}
              <div className={styles.actions}>
                <Button
                  size="sm"
                  variant="soft"
                  disabled={pending}
                  onClick={() => void reload()}
                >
                  Reload board
                </Button>
                <Button
                  size="sm"
                  variant="soft"
                  disabled={!canReview || isLoading}
                  onClick={() => {
                    setReviewedRev(board.rev);
                    setConflict(null);
                    setError(null);
                  }}
                >
                  Use revision {board.rev} after review
                </Button>
              </div>
            </section>
          )}
          {boardUnavailable && (
            <div role="alert">
              The current board could not be loaded. Your draft is kept; reload
              before saving.
            </div>
          )}
          {reloading && <div role="status">Reloading board for review...</div>}
          {editing ? (
            <form
              id={fieldId}
              className={styles.form}
              onSubmit={(event) => {
                event.preventDefault();
                void submit();
              }}
            >
              <Field
                label="Card ID"
                htmlFor={`${fieldId}-id`}
                helper={
                  creating
                    ? "Unique within this task."
                    : "Card IDs cannot be changed."
                }
              >
                <FieldText
                  id={`${fieldId}-id`}
                  value={draft.id}
                  readOnly={!creating}
                  disabled={isLoading}
                  onChange={(id) => setDraft((value) => ({ ...value, id }))}
                />
              </Field>
              <Field label="Title" htmlFor={`${fieldId}-title`}>
                <FieldText
                  id={`${fieldId}-title`}
                  value={draft.title}
                  disabled={isLoading}
                  onChange={(title) =>
                    setDraft((value) => ({ ...value, title }))
                  }
                />
              </Field>
              <Field label="Priority" htmlFor={`${fieldId}-priority`}>
                <FieldSelect
                  id={`${fieldId}-priority`}
                  value={draft.priority}
                  disabled={isLoading}
                  options={[...new Set(["P0", "P1", "P2", draft.priority])].map(
                    (value) => ({ value, label: value }),
                  )}
                  onChange={(priority) =>
                    setDraft((value) => ({ ...value, priority }))
                  }
                />
              </Field>
              <Field label="Instructions" htmlFor={`${fieldId}-instructions`}>
                <FieldTextarea
                  id={`${fieldId}-instructions`}
                  rows={5}
                  value={draft.instructions}
                  disabled={isLoading}
                  onChange={(instructions) =>
                    setDraft((value) => ({ ...value, instructions }))
                  }
                />
              </Field>
              <Field
                label="Target files"
                htmlFor={`${fieldId}-files`}
                helper="One path per line. Declared intent only, not an enforced boundary."
              >
                <FieldTextarea
                  id={`${fieldId}-files`}
                  rows={3}
                  value={draft.targetFiles}
                  disabled={isLoading}
                  onChange={(targetFiles) =>
                    setDraft((value) => ({ ...value, targetFiles }))
                  }
                />
              </Field>
              <fieldset className={styles.dependencies}>
                <legend>Dependencies</legend>
                <div className={styles.hint}>
                  Choices come from the visible current board. Clear all to
                  remove dependencies. The server remains authoritative.
                </div>
                {dependencyChoices.length === 0 && (
                  <div>No other visible cards.</div>
                )}
                <div className={styles.dependencyList}>
                  {dependencyChoices.map((card) => (
                    <Checkbox
                      key={card.id}
                      checked={draft.dependsOn.includes(card.id)}
                      disabled={isLoading}
                      onCheckedChange={(checked) =>
                        setDraft((value) => ({
                          ...value,
                          dependsOn:
                            checked === true
                              ? [...value.dependsOn, card.id]
                              : value.dependsOn.filter((id) => id !== card.id),
                        }))
                      }
                    >
                      {card.id}: {card.title}
                    </Checkbox>
                  ))}
                  {unknownDependencies.map((id) => (
                    <Checkbox
                      key={id}
                      checked
                      disabled={isLoading}
                      onCheckedChange={() =>
                        setDraft((value) => ({
                          ...value,
                          dependsOn: value.dependsOn.filter(
                            (valueId) => valueId !== id,
                          ),
                        }))
                      }
                    >
                      {id} (unavailable dependency; uncheck to remove)
                    </Checkbox>
                  ))}
                </div>
                <Button
                  size="sm"
                  variant="soft"
                  disabled={isLoading || draft.dependsOn.length === 0}
                  onClick={() =>
                    setDraft((value) => ({ ...value, dependsOn: [] }))
                  }
                >
                  Clear dependencies
                </Button>
              </fieldset>
            </form>
          ) : target.mode === "move" ? (
            <>
              <Field label="Move to column" htmlFor={`${fieldId}-column`}>
                <FieldSelect
                  id={`${fieldId}-column`}
                  value={column}
                  disabled={isLoading}
                  options={MOVE_COLUMNS.map((value) => ({
                    value,
                    label:
                      board.columns.find((item) => item.id === value)?.title ??
                      humanizeIdentifier(value),
                  }))}
                  onChange={setColumn}
                />
              </Field>
              <div className={styles.hint}>
                Moving changes workflow metadata only. It does not start or stop
                an agent, verify work, or finalize a card. Done does not mean
                verified. Server guards may refuse the move.
              </div>
            </>
          ) : (
            <div>
              Delete this card permanently? This sends a nonforce request only.
              Doing cards and cards with agent references remain protected; this
              control never terminates an agent.
            </div>
          )}
        </div>
        <Dialog.Footer className={styles.actions}>
          <Button variant="soft" disabled={pending} onClick={onClose}>
            Cancel
          </Button>
          <Button
            type={editing ? "submit" : "button"}
            form={editing ? fieldId : undefined}
            variant={target.mode === "delete" ? "danger" : "primary"}
            disabled={
              pending || needsReview || boardUnavailable || boardFetching
            }
            loading={isLoading}
            onClick={editing ? undefined : () => void submit()}
          >
            {isLoading
              ? "Saving..."
              : title === "Edit card"
                ? "Save card"
                : title}
          </Button>
        </Dialog.Footer>
      </Dialog.Content>
    </Dialog>
  );
}
