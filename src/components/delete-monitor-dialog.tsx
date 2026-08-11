import { useRef } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useDeleteMonitor } from "@/lib/queries";
import type { Monitor } from "@/lib/tauri";

interface DeleteMonitorDialogProps {
  monitor: Monitor | null;
  onOpenChange: (open: boolean) => void;
}

export function DeleteMonitorDialog({
  monitor,
  onOpenChange,
}: DeleteMonitorDialogProps) {
  const deleteMonitor = useDeleteMonitor({
    onSuccess: () => onOpenChange(false),
  });

  // Keep the last monitor around so the text doesn't blank out during the
  // close animation.
  const lastMonitor = useRef(monitor);
  if (monitor) lastMonitor.current = monitor;
  const shown = monitor ?? lastMonitor.current;

  return (
    <Dialog open={monitor !== null} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Delete monitor?</DialogTitle>
          <DialogDescription>
            {shown?.url} will no longer be checked, and all of its recorded
            history will be removed with it. This cannot be undone.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            disabled={deleteMonitor.isPending}
            onClick={() => monitor && deleteMonitor.mutate(monitor.id)}
          >
            Delete
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
