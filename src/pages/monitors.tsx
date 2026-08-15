import { useState } from "react";
import { Pencil, Plus, RefreshCw, Trash2, Upload } from "lucide-react";
import { toast } from "sonner";

import { DeleteMonitorDialog } from "@/components/delete-monitor-dialog";
import { DeleteMonitorsDialog } from "@/components/delete-monitors-dialog";
import { ImportMonitorsDialog } from "@/components/import-monitors-dialog";
import { MonitorFormDialog } from "@/components/monitor-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { errorMessage } from "@/lib/errors";
import { formatMs, formatTimeAgo } from "@/lib/format";
import { monitorToInput } from "@/lib/monitor-form";
import {
  useCheckMonitors,
  useCheckNow,
  useMonitors,
  useSetMonitorsEnabled,
  useUpdateMonitor,
} from "@/lib/queries";
import { pickMonitorSyncFile } from "@/lib/tauri";
import type { Monitor, UptimeStatus } from "@/lib/tauri";

const STATUS_LABELS: Record<UptimeStatus, string> = {
  not_yet_checked: "Not checked yet",
  up: "Up",
  down: "Down",
};

const STATUS_VARIANTS: Record<
  UptimeStatus,
  "default" | "secondary" | "destructive"
> = {
  not_yet_checked: "secondary",
  up: "default",
  down: "destructive",
};

function StatusBadge({ monitor }: { monitor: Monitor }) {
  const status: UptimeStatus = monitor.uptimeStatus;
  return (
    <Badge
      variant={STATUS_VARIANTS[status]}
      title={monitor.uptimeFailureReason ?? undefined}
    >
      {STATUS_LABELS[status]}
    </Badge>
  );
}

function CertificateBadge({ monitor }: { monitor: Monitor }) {
  if (!monitor.certCheckEnabled) return <span>Off</span>;
  if (monitor.certStatus === "invalid") {
    return <Badge variant="destructive">Certificate issue</Badge>;
  }
  if (monitor.certStatus === "not_yet_checked") {
    return <span>Not checked</span>;
  }
  const expiresAt = monitor.certExpiresAt
    ? new Date(monitor.certExpiresAt).getTime()
    : Number.NaN;
  const daysRemaining = Math.ceil((expiresAt - Date.now()) / 86_400_000);
  if (Number.isFinite(daysRemaining) && daysRemaining <= 10) {
    return (
      <Badge variant="outline" className="border-amber-500 text-amber-700">
        Expires in {Math.max(0, daysRemaining)}d
      </Badge>
    );
  }
  return <span>Valid</span>;
}

export function MonitorsPage() {
  const monitors = useMonitors();
  const updateMonitor = useUpdateMonitor();
  const checkNow = useCheckNow();
  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<Monitor | undefined>(undefined);
  const [deleting, setDeleting] = useState<Monitor | null>(null);
  const [syncPath, setSyncPath] = useState<string | null>(null);
  const [deleteMissing, setDeleteMissing] = useState(false);
  // Selection is view-local: it never outlives the page or a bulk action.
  const [selected, setSelected] = useState<number[]>([]);
  const [bulkDeleting, setBulkDeleting] = useState<number[]>([]);

  const rows = monitors.data ?? [];
  const selectedIds = selected.filter((id) =>
    rows.some((monitor) => monitor.id === id),
  );
  const allSelected = rows.length > 0 && selectedIds.length === rows.length;

  const setMonitorsEnabled = useSetMonitorsEnabled();
  const checkMonitors = useCheckMonitors({
    onSuccess: (summary) => {
      toast.success(
        `Checked ${summary.checked} ${summary.checked === 1 ? "monitor" : "monitors"}: ${summary.up} up, ${summary.down} down.`,
      );
    },
  });
  // Only selection-scoped actions clear the selection, and they clear it
  // whether the call succeeded or failed.
  const clearSelection = { onSettled: () => setSelected([]) };
  const checkingIds = checkMonitors.isPending
    ? (checkMonitors.variables ?? [])
    : [];

  const toggleSelected = (id: number, checked: boolean) =>
    setSelected((current) =>
      checked ? [...current, id] : current.filter((other) => other !== id),
    );
  // "Check all" means every enabled monitor: forcing a check overrides the
  // interval, not the enabled flag. An explicit selection is checked as-is.
  const enabledIds = rows
    .filter((monitor) => monitor.uptimeCheckEnabled)
    .map((monitor) => monitor.id);

  // The picker is a native dialog, not a data command: it hands back a path
  // that the preview/apply commands read on the Rust side.
  const openImport = async () => {
    try {
      const path = await pickMonitorSyncFile();
      if (path === null) return;
      setDeleteMissing(false);
      setSyncPath(path);
    } catch (error) {
      toast.error(errorMessage(error));
    }
  };

  const openAdd = () => {
    setEditing(undefined);
    setFormOpen(true);
  };
  const openEdit = (monitor: Monitor) => {
    setEditing(monitor);
    setFormOpen(true);
  };
  const toggleEnabled = (monitor: Monitor, enabled: boolean) => {
    updateMonitor.mutate({
      id: monitor.id,
      input: { ...monitorToInput(monitor), uptimeCheckEnabled: enabled },
    });
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold tracking-tight">Monitors</h1>
        <div className="flex gap-2">
          <Button variant="outline" onClick={openImport}>
            <Upload /> Import / Sync
          </Button>
          {rows.length > 0 && (
            <>
              <Button
                variant="outline"
                onClick={() => checkMonitors.mutate(enabledIds)}
                disabled={checkMonitors.isPending || enabledIds.length === 0}
              >
                <RefreshCw
                  className={checkMonitors.isPending ? "animate-spin" : undefined}
                />
                Check all now
              </Button>
              <Button onClick={openAdd}>
                <Plus /> Add monitor
              </Button>
            </>
          )}
        </div>
      </div>

      {monitors.isPending ? (
        <p className="text-sm text-muted-foreground">Loading…</p>
      ) : monitors.isError ? (
        <p className="text-sm text-destructive">
          Could not load monitors. {String(monitors.error)}
        </p>
      ) : monitors.data.length === 0 ? (
        <div className="flex flex-col items-center gap-4 rounded-lg border border-dashed py-16 text-center">
          <p className="text-sm text-muted-foreground">
            No monitors yet. Add the first URL you want to keep an eye on.
          </p>
          <Button onClick={openAdd}>
            <Plus /> Add your first monitor
          </Button>
        </div>
      ) : (
        <>
        {selectedIds.length > 0 && (
          <div className="flex items-center gap-2 rounded-lg border bg-muted/40 px-4 py-2">
            <span className="text-sm font-medium">
              {selectedIds.length} selected
            </span>
            <div className="ml-auto flex gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={checkMonitors.isPending}
                onClick={() =>
                  checkMonitors.mutate(selectedIds, clearSelection)
                }
              >
                Check selected now
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={setMonitorsEnabled.isPending}
                onClick={() =>
                  setMonitorsEnabled.mutate(
                    { ids: selectedIds, enabled: true },
                    clearSelection,
                  )
                }
              >
                Enable
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={setMonitorsEnabled.isPending}
                onClick={() =>
                  setMonitorsEnabled.mutate(
                    { ids: selectedIds, enabled: false },
                    clearSelection,
                  )
                }
              >
                Disable
              </Button>
              <Button
                variant="destructive"
                size="sm"
                onClick={() => setBulkDeleting(selectedIds)}
              >
                Delete
              </Button>
            </div>
          </div>
        )}
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="w-8">
                <Checkbox
                  aria-label="Select all monitors"
                  checked={
                    allSelected
                      ? true
                      : selectedIds.length > 0
                        ? "indeterminate"
                        : false
                  }
                  onCheckedChange={(checked) =>
                    setSelected(checked === true ? rows.map((m) => m.id) : [])
                  }
                />
              </TableHead>
              <TableHead>URL</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Last check</TableHead>
              <TableHead>Response</TableHead>
              <TableHead>Interval</TableHead>
              <TableHead>Method</TableHead>
              <TableHead>Certificate</TableHead>
              <TableHead>Enabled</TableHead>
              <TableHead className="w-32" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {monitors.data.map((monitor) => (
              <TableRow key={monitor.id}>
                <TableCell>
                  <Checkbox
                    aria-label={`Select ${monitor.url}`}
                    checked={selectedIds.includes(monitor.id)}
                    onCheckedChange={(checked) =>
                      toggleSelected(monitor.id, checked === true)
                    }
                  />
                </TableCell>
                <TableCell className="font-medium">
                  <span className="flex items-center gap-2">
                    {checkingIds.includes(monitor.id) && (
                      <RefreshCw
                        className="size-3.5 animate-spin text-muted-foreground"
                        aria-label={`Checking ${monitor.url}`}
                      />
                    )}
                    {monitor.url}
                  </span>
                </TableCell>
                <TableCell>
                  <StatusBadge monitor={monitor} />
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {formatTimeAgo(monitor.lastCheckAt)}
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {formatMs(monitor.lastResponseTimeMs)}
                </TableCell>
                <TableCell>{monitor.checkIntervalMinutes} min</TableCell>
                <TableCell>{monitor.checkMethod}</TableCell>
                <TableCell className="text-muted-foreground">
                  <CertificateBadge monitor={monitor} />
                </TableCell>
                <TableCell>
                  <Switch
                    aria-label={`Uptime checks for ${monitor.url}`}
                    checked={monitor.uptimeCheckEnabled}
                    onCheckedChange={(v) => toggleEnabled(monitor, v)}
                  />
                </TableCell>
                <TableCell>
                  <div className="flex justify-end gap-1">
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label={`Check ${monitor.url} now`}
                      disabled={
                        checkNow.isPending && checkNow.variables === monitor.id
                      }
                      onClick={() => checkNow.mutate(monitor.id)}
                    >
                      <RefreshCw
                        className={
                          checkNow.isPending && checkNow.variables === monitor.id
                            ? "animate-spin"
                            : undefined
                        }
                      />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label={`Edit ${monitor.url}`}
                      onClick={() => openEdit(monitor)}
                    >
                      <Pencil />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label={`Delete ${monitor.url}`}
                      onClick={() => setDeleting(monitor)}
                    >
                      <Trash2 />
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
        </>
      )}

      <MonitorFormDialog
        open={formOpen}
        onOpenChange={setFormOpen}
        monitor={editing}
      />
      <DeleteMonitorDialog
        monitor={deleting}
        onOpenChange={(open) => !open && setDeleting(null)}
      />
      <DeleteMonitorsDialog
        ids={bulkDeleting}
        onOpenChange={(open) => !open && setBulkDeleting([])}
        onDeleted={() => setSelected([])}
      />
      <ImportMonitorsDialog
        path={syncPath}
        onOpenChange={(open) => !open && setSyncPath(null)}
        deleteMissing={deleteMissing}
        onDeleteMissingChange={setDeleteMissing}
      />
    </div>
  );
}
