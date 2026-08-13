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
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { errorMessage } from "@/lib/errors";
import { useApplyMonitorSync, useMonitorSyncPreview } from "@/lib/queries";
import type { SyncPlan } from "@/lib/tauri";

interface ImportMonitorsDialogProps {
  /** Path of the chosen file; null keeps the dialog closed. */
  path: string | null;
  onOpenChange: (open: boolean) => void;
  deleteMissing: boolean;
  onDeleteMissingChange: (deleteMissing: boolean) => void;
}

function PlanGroup({
  title,
  urls,
  note,
}: {
  title: string;
  urls: string[];
  note?: string;
}) {
  if (urls.length === 0) return null;
  return (
    <section className="space-y-1">
      <h3 className="text-sm font-medium">
        {title} ({urls.length})
      </h3>
      {note && <p className="text-xs text-muted-foreground">{note}</p>}
      <ul className="max-h-32 overflow-y-auto text-sm text-muted-foreground">
        {urls.map((url) => (
          <li key={url}>{url}</li>
        ))}
      </ul>
    </section>
  );
}

function summarize(plan: SyncPlan): string {
  const groups = [
    `${plan.toAdd.length} to add`,
    `${plan.toUpdate.length} to update`,
    ...(plan.deleteMissing ? [`${plan.toDelete.length} to delete`] : []),
    `${plan.unchanged.length} unchanged`,
  ];
  return groups.join(", ");
}

export function ImportMonitorsDialog({
  path,
  onOpenChange,
  deleteMissing,
  onDeleteMissingChange,
}: ImportMonitorsDialogProps) {
  const preview = useMonitorSyncPreview(path, deleteMissing);
  const applySync = useApplyMonitorSync({
    onSuccess: (result) => {
      toast.success(
        `Sync applied: ${result.added} added, ${result.updated} updated, ${result.deleted} deleted.`,
      );
      onOpenChange(false);
    },
  });

  const plan = preview.data;
  const nothingToDo =
    plan !== undefined &&
    plan.toAdd.length === 0 &&
    plan.toUpdate.length === 0 &&
    plan.toDelete.length === 0;

  return (
    <Dialog open={path !== null} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Import monitors</DialogTitle>
          <DialogDescription>
            {preview.isPending
              ? "Reading the file…"
              : preview.isError
                ? "The file could not be imported."
                : plan
                  ? summarize(plan)
                  : ""}
          </DialogDescription>
        </DialogHeader>

        {preview.isError ? (
          <p className="text-sm text-destructive">
            {errorMessage(preview.error)}
          </p>
        ) : plan ? (
          <div className="space-y-4">
            <PlanGroup title="Add" urls={plan.toAdd} />
            <PlanGroup title="Update" urls={plan.toUpdate} />
            <PlanGroup
              title="Delete"
              urls={plan.toDelete}
              note="Their recorded check history is removed with them."
            />
            <PlanGroup title="Unchanged" urls={plan.unchanged} />
            {nothingToDo && (
              <p className="text-sm text-muted-foreground">
                This file matches the current monitors. Nothing to apply.
              </p>
            )}
          </div>
        ) : null}

        <div className="flex items-center gap-2">
          <Switch
            id="delete-missing"
            checked={deleteMissing}
            onCheckedChange={onDeleteMissingChange}
          />
          <Label htmlFor="delete-missing">
            Delete monitors missing from the file
          </Label>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            disabled={
              path === null ||
              plan === undefined ||
              nothingToDo ||
              applySync.isPending
            }
            onClick={() =>
              path && applySync.mutate({ path, deleteMissing })
            }
          >
            Apply
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
