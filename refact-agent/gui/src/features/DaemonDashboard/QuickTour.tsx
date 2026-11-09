import { useState } from "react";
import { Button, Dialog } from "../../components/ui";
import { useAppDispatch } from "../../hooks";
import { navigateDashboard, type DashboardPage } from "./dashboardSlice";

const STEPS: { page: DashboardPage; title: string; description: string }[] = [
  {
    page: "projects",
    title: "Projects",
    description:
      "Your project workspaces live here. Opening a project is an explicit action; passive console observation never starts workers.",
  },
  {
    page: "engines",
    title: "Engines",
    description:
      "See managed workers and connect existing numeric-loopback engines. Attachments are read-only and clear on daemon restart.",
  },
  {
    page: "activity",
    title: "Logs",
    description:
      "Inspect daemon and managed project logs, pause the display or download a snapshot. Attached engine live logs are unavailable.",
  },
  {
    page: "chats",
    title: "Active chats",
    description:
      "Filter loaded sessions across engines. Unknown or unsupported coverage is shown explicitly, not as a successful zero count.",
  },
];
export function QuickTour() {
  const dispatch = useAppDispatch();
  const [open, setOpen] = useState(false);
  const [step, setStep] = useState(0);
  const go = (index: number) => {
    setStep(index);
    dispatch(navigateDashboard({ page: STEPS[index].page, params: {} }));
  };
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <Dialog.Trigger asChild>
        <Button variant="ghost" size="sm" onClick={() => go(0)}>
          Quick tour
        </Button>
      </Dialog.Trigger>
      <Dialog.Content>
        <Dialog.Title>
          {STEPS[step].title} · {step + 1} of {STEPS.length}
        </Dialog.Title>
        <Dialog.Description>{STEPS[step].description}</Dialog.Description>
        <Dialog.Footer>
          <Button variant="ghost" onClick={() => setOpen(false)}>
            Skip
          </Button>
          <Button
            variant="ghost"
            disabled={step === 0}
            onClick={() => go(step - 1)}
          >
            Back
          </Button>
          <Button
            onClick={() =>
              step === STEPS.length - 1 ? setOpen(false) : go(step + 1)
            }
          >
            {step === STEPS.length - 1 ? "Finish" : "Next"}
          </Button>
        </Dialog.Footer>
      </Dialog.Content>
    </Dialog>
  );
}
