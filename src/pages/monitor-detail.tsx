import { useState } from "react";
import { ArrowLeft, Pencil, RefreshCw, Trash2 } from "lucide-react";
import { Link, useNavigate, useParams } from "react-router";
import {
  CartesianGrid,
  Line,
  LineChart,
  ReferenceArea,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import { DeleteMonitorDialog } from "@/components/delete-monitor-dialog";
import { MonitorFormDialog } from "@/components/monitor-form-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { certificateState } from "@/lib/certificate";
import { formatDuration, formatLocalDateTime, formatMs } from "@/lib/format";
import { useCheckNow, useHistory, useMonitor, useUptimeStats } from "@/lib/queries";
import type { HistoryPoint, HistoryRange, Monitor, UptimeStatus } from "@/lib/tauri";
import { cn } from "@/lib/utils";

const RANGES: { value: HistoryRange; label: string }[] = [
  { value: "24h", label: "24 hours" },
  { value: "7d", label: "7 days" },
  { value: "30d", label: "30 days" },
];

const STATUS_BADGE: Record<UptimeStatus, "default" | "secondary" | "destructive"> = {
  up: "default",
  down: "destructive",
  not_yet_checked: "secondary",
};

function rangeColor(status: HistoryPoint["status"]): string {
  switch (status) {
    case "up":
      return "bg-emerald-500";
    case "down":
      return "bg-red-500";
    case "mixed":
      return "bg-[repeating-linear-gradient(135deg,#22c55e_0_4px,#ef4444_4px_8px)]";
    case "gap":
      return "bg-zinc-400";
  }
}

function UptimeBar({ points }: { points: HistoryPoint[] }) {
  if (points.length === 0) {
    return <div className="h-5 rounded-full bg-zinc-300" title="No data" />;
  }
  return (
    <div className="flex h-5 overflow-hidden rounded-full bg-zinc-300" aria-label="Uptime timeline">
      {points.map((point) => {
        const duration = Math.max(
          1,
          new Date(point.endedAt).getTime() - new Date(point.startedAt).getTime(),
        );
        return (
          <span
            key={`${point.startedAt}-${point.status}`}
            className={rangeColor(point.status)}
            style={{ flexGrow: duration, flexBasis: 0 }}
            title={`${point.status} · ${formatLocalDateTime(point.startedAt)}`}
          />
        );
      })}
    </div>
  );
}

function LatencyChart({ points }: { points: HistoryPoint[] }) {
  const data = points.map((point) => ({
    timestamp: new Date(point.startedAt).getTime(),
    responseTime: point.status === "gap" ? null : point.avgResponseTimeMs,
  }));
  if (points.length === 0) {
    return (
      <div className="flex h-64 items-center justify-center rounded-lg border border-dashed text-sm text-muted-foreground">
        No latency data in this range.
      </div>
    );
  }
  return (
    <div className="h-64 w-full">
      <ResponsiveContainer width="100%" height="100%">
        <LineChart data={data} margin={{ top: 8, right: 12, bottom: 4, left: 0 }}>
          <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
          <XAxis
            dataKey="timestamp"
            type="number"
            domain={["dataMin", "dataMax"]}
            tickFormatter={(value) => new Date(value).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}
            className="text-xs"
          />
          <YAxis
            width={56}
            tickFormatter={(value) => `${value} ms`}
            className="text-xs"
          />
          <Tooltip
            labelFormatter={(value) => formatLocalDateTime(new Date(Number(value)).toISOString())}
            formatter={(value) => [formatMs(Number(value)), "Response"]}
          />
          {points
            .filter((point) => point.status === "gap" || point.status === "mixed")
            .map((point) => (
              <ReferenceArea
                key={point.startedAt}
                x1={new Date(point.startedAt).getTime()}
                x2={new Date(point.endedAt).getTime()}
                fill="#a1a1aa"
                fillOpacity={0.22}
                strokeOpacity={0}
              />
            ))}
          <Line
            dataKey="responseTime"
            type="linear"
            stroke="#0ea5e9"
            strokeWidth={2}
            dot={false}
            connectNulls={false}
            isAnimationActive={false}
          />
        </LineChart>
      </ResponsiveContainer>
    </div>
  );
}

function CertificatePanel({ monitor }: { monitor: Monitor }) {
  const state = certificateState(monitor);
  if (state.kind === "disabled") {
    return <p className="text-sm text-muted-foreground">Certificate checks are disabled.</p>;
  }
  if (state.kind === "not_checked") {
    return <p className="text-sm text-muted-foreground">Not checked yet</p>;
  }
  const expiry = state.expiresAt;
  return (
    <dl className="grid gap-4 text-sm sm:grid-cols-3">
      <div>
        <dt className="text-muted-foreground">Status</dt>
        <dd className={cn("mt-1 font-medium", state.kind === "invalid" && "text-destructive")}>
          {state.kind === "invalid" ? "Invalid" : "Valid"}
        </dd>
      </div>
      <div>
        <dt className="text-muted-foreground">Issuer</dt>
        <dd className="mt-1 font-medium">{monitor.certIssuer ?? "—"}</dd>
      </div>
      <div>
        <dt className="text-muted-foreground">Expires</dt>
        <dd
          className={cn(
            "mt-1 font-medium",
            state.kind === "expiring_soon" && "text-amber-600",
            state.kind === "expired" && "text-destructive",
          )}
        >
          {expiry ? expiry.toLocaleDateString() : "—"}
          {state.kind === "expiring_soon" && ` · ${state.daysRemaining} days`}
          {state.kind === "expired" && ` · expired ${state.daysAgo} days ago`}
        </dd>
      </div>
      {monitor.certFailureReason && (
        <p className="text-destructive sm:col-span-3">{monitor.certFailureReason}</p>
      )}
    </dl>
  );
}

export function MonitorDetailPage() {
  const params = useParams();
  const navigate = useNavigate();
  const id = Number(params.id);
  const validId = Number.isSafeInteger(id) && id > 0;
  const monitor = useMonitor(validId ? id : 0);
  const stats = useUptimeStats(validId ? id : 0);
  const [range, setRange] = useState<HistoryRange>("24h");
  const history = useHistory(validId ? id : 0, range);
  const checkNow = useCheckNow();
  const [editing, setEditing] = useState(false);
  const [deleting, setDeleting] = useState(false);

  if (!validId) {
    return <p className="text-sm text-destructive">Invalid monitor id.</p>;
  }
  if (monitor.isPending) {
    return <p className="text-sm text-muted-foreground">Loading monitor…</p>;
  }
  if (monitor.isError) {
    return <p className="text-sm text-destructive">Could not load this monitor.</p>;
  }
  const value = monitor.data;

  return (
    <div className="space-y-6">
      <Link to="/" className="inline-flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground">
        <ArrowLeft className="size-4" /> Dashboard
      </Link>

      <header className="flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-3">
            <Badge variant={STATUS_BADGE[value.uptimeStatus]}>{value.uptimeStatus.replace(/_/g, " ")}</Badge>
            <span className="text-sm text-muted-foreground">
              {value.checkMethod} every {value.checkIntervalMinutes} min
            </span>
          </div>
          <h1 className="mt-3 truncate text-2xl font-bold tracking-tight" title={value.url}>
            {value.url}
          </h1>
          {value.uptimeFailureReason && (
            <p className="mt-1 text-sm text-destructive">{value.uptimeFailureReason}</p>
          )}
        </div>
        <div className="flex gap-2">
          <Button
            variant="outline"
            disabled={checkNow.isPending}
            onClick={() => checkNow.mutate(value.id)}
          >
            <RefreshCw className={checkNow.isPending ? "animate-spin" : undefined} /> Check now
          </Button>
          <Button variant="outline" size="icon" aria-label="Edit monitor" onClick={() => setEditing(true)}>
            <Pencil />
          </Button>
          <Button variant="outline" size="icon" aria-label="Delete monitor" onClick={() => setDeleting(true)}>
            <Trash2 />
          </Button>
        </div>
      </header>

      <div className="grid gap-4 sm:grid-cols-4">
        {[
          ["24 hours", stats.data?.uptime24h],
          ["7 days", stats.data?.uptime7d],
          ["30 days", stats.data?.uptime30d],
        ].map(([label, uptime]) => (
          <Card key={String(label)} className="gap-2 p-5">
            <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">{label}</p>
            <p className="text-2xl font-semibold tabular-nums">
              {typeof uptime === "number" ? `${uptime.toFixed(1)}%` : "No data"}
            </p>
          </Card>
        ))}
        <Card className="gap-2 p-5">
          <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Avg latency · 24h</p>
          <p className="text-2xl font-semibold tabular-nums">{formatMs(stats.data?.avgResponseTimeMs24h ?? null)}</p>
        </Card>
      </div>

      <Card>
        <CardHeader className="flex-row items-center justify-between">
          <div>
            <CardTitle>Latency</CardTitle>
            <CardDescription>Response time; gray spans are monitoring gaps.</CardDescription>
          </div>
          <div className="flex rounded-md border p-1">
            {RANGES.map((option) => (
              <Button
                key={option.value}
                size="sm"
                variant={range === option.value ? "secondary" : "ghost"}
                aria-label={option.label}
                onClick={() => setRange(option.value)}
              >
                {option.value}
              </Button>
            ))}
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          {history.isError ? (
            <p className="text-sm text-destructive">Could not load history.</p>
          ) : (
            <>
              <LatencyChart points={history.data?.points ?? []} />
              <UptimeBar points={history.data?.points ?? []} />
              <div className="flex gap-4 text-xs text-muted-foreground">
                <span><i className="mr-1 inline-block size-2 rounded-full bg-emerald-500" />Up</span>
                <span><i className="mr-1 inline-block size-2 rounded-full bg-red-500" />Down</span>
                <span><i className="mr-1 inline-block size-2 rounded-full bg-zinc-400" />Gap / no data</span>
              </div>
            </>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Certificate</CardTitle>
          <CardDescription>TLS certificate status for this endpoint.</CardDescription>
        </CardHeader>
        <CardContent><CertificatePanel monitor={value} /></CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Incidents</CardTitle>
          <CardDescription>Contiguous periods with failed uptime checks.</CardDescription>
        </CardHeader>
        <CardContent>
          {(history.data?.incidents.length ?? 0) === 0 ? (
            <p className="text-sm text-muted-foreground">No incidents in this range.</p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Started</TableHead>
                  <TableHead>Ended</TableHead>
                  <TableHead>Duration</TableHead>
                  <TableHead>Reason</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {history.data?.incidents.map((incident) => (
                  <TableRow key={incident.startedAt}>
                    <TableCell>{formatLocalDateTime(incident.startedAt)}</TableCell>
                    <TableCell>{incident.ongoing ? <Badge variant="destructive">Ongoing</Badge> : formatLocalDateTime(incident.endedAt)}</TableCell>
                    <TableCell>
                      {formatDuration(incident.durationSeconds)}
                      {incident.includesGap && <span className="block text-xs text-muted-foreground">Includes monitoring gap</span>}
                    </TableCell>
                    <TableCell className="max-w-80 truncate" title={incident.failureReason ?? undefined}>
                      {incident.failureReason ?? "—"}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      <MonitorFormDialog open={editing} onOpenChange={setEditing} monitor={value} />
      <DeleteMonitorDialog
        monitor={deleting ? value : null}
        onOpenChange={setDeleting}
        onDeleted={() => navigate("/")}
      />
    </div>
  );
}
