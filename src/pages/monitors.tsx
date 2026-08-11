import { useState } from "react";
import { Pencil, Plus, Trash2 } from "lucide-react";

import { DeleteMonitorDialog } from "@/components/delete-monitor-dialog";
import { MonitorFormDialog } from "@/components/monitor-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { monitorToInput } from "@/lib/monitor-form";
import { useMonitors, useUpdateMonitor } from "@/lib/queries";
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

function StatusBadge({ status }: { status: UptimeStatus }) {
  return <Badge variant={STATUS_VARIANTS[status]}>{STATUS_LABELS[status]}</Badge>;
}

export function MonitorsPage() {
  const monitors = useMonitors();
  const updateMonitor = useUpdateMonitor();
  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<Monitor | undefined>(undefined);
  const [deleting, setDeleting] = useState<Monitor | null>(null);

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
        {(monitors.data?.length ?? 0) > 0 && (
          <Button onClick={openAdd}>
            <Plus /> Add monitor
          </Button>
        )}
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
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>URL</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Interval</TableHead>
              <TableHead>Method</TableHead>
              <TableHead>Certificate</TableHead>
              <TableHead>Enabled</TableHead>
              <TableHead className="w-24" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {monitors.data.map((monitor) => (
              <TableRow key={monitor.id}>
                <TableCell className="font-medium">{monitor.url}</TableCell>
                <TableCell>
                  <StatusBadge status={monitor.uptimeStatus} />
                </TableCell>
                <TableCell>{monitor.checkIntervalMinutes} min</TableCell>
                <TableCell>{monitor.checkMethod}</TableCell>
                <TableCell className="text-muted-foreground">
                  {monitor.certCheckEnabled ? "Checked" : "Off"}
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
    </div>
  );
}
