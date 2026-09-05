import { useState } from "react";
import { useAppSelector } from "../../hooks";
import { selectContextRebuildMessageById } from "../../features/Chat/Thread/selectors";
import { Popover } from "../LongTailPrimitives";
import { Button } from "../ui";
import { TrajectoryPopoverContent } from "../Trajectory/TrajectoryPopover";

export function ContextRebuildBanner({ chatId }: { chatId: string }) {
  const message = useAppSelector((state) =>
    selectContextRebuildMessageById(state, chatId),
  );
  const [open, setOpen] = useState(false);
  if (!message) return null;
  return (
    <div role="alert">
      <p>{message} Original messages remain available below.</p>
      <Popover.Root open={open} onOpenChange={setOpen}>
        <Popover.Trigger asChild>
          <Button size="sm">Rebuild context</Button>
        </Popover.Trigger>
        <TrajectoryPopoverContent
          initialTab="llm-compress"
          chatId={chatId}
          onClose={() => setOpen(false)}
        />
      </Popover.Root>
    </div>
  );
}
