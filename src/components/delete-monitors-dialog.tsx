import { useRef } from "react";

import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useDeleteMonitors } from "@/lib/queries";

interface DeleteMonitorsDialogProps {
  /** Ids to delete; an empty array keeps the dialog closed. */
  ids: number[];
  onOpenChange: (open: boolean) => void;
  onDeleted?: () => void;
}

export function DeleteMonitorsDialog({
  ids,
  onOpenChange,
  onDeleted,
}: DeleteMonitorsDialogProps) {
  const deleteMonitors = useDeleteMonitors({
    onSuccess: (deleted) => {
      toast.success(
        `Deleted ${deleted} ${deleted === 1 ? "monitor" : "monitors"}.`,
      );
      onOpenChange(false);
      onDeleted?.();
    },
  });

  // Keep the last count so the text doesn't blank out during the close
  // animation.
  const lastCount = useRef(ids.length);
  if (ids.length > 0) lastCount.current = ids.length;
  const count = ids.length > 0 ? ids.length : lastCount.current;

  return (
    <Dialog open={ids.length > 0} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            Delete {count} {count === 1 ? "monitor" : "monitors"}?
          </DialogTitle>
          <DialogDescription>
            They will no longer be checked, and all of their recorded history
            will be removed with them. This cannot be undone.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            disabled={deleteMonitors.isPending}
            onClick={() => ids.length > 0 && deleteMonitors.mutate(ids)}
          >
            Delete
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
