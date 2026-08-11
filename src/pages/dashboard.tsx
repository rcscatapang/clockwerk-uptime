import { Activity, ArrowRight } from "lucide-react";
import { Link } from "react-router";

import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { formatMs, formatTimeAgo } from "@/lib/format";
import { useHistory, useMonitors, useUptimeStats } from "@/lib/queries";
import type { HistoryPoint, Monitor, UptimeStatus } from "@/lib/tauri";
import { cn } from "@/lib/utils";

const STATUS_ORDER: Record<UptimeStatus, number> = {
  down: 0,
  not_yet_checked: 1,
  up: 2,
};

const STATUS_LABEL: Record<UptimeStatus, string> = {
  down: "Down",
  not_yet_checked: "Not checked",
  up: "Up",
};

function StatusDot({ status }: { status: UptimeStatus }) {
  return (
    <span className="inline-flex items-center gap-2 text-xs font-medium">
      <span
        aria-hidden="true"
        className={cn(
          "size-2 rounded-full",
          status === "up" && "bg-emerald-500",
          status === "down" && "bg-red-500",
          status === "not_yet_checked" && "bg-zinc-400",
        )}
      />
      {STATUS_LABEL[status]}
    </span>
  );
}

function Stat({ label, value }: { label: string; value: number | null | undefined }) {
  return (
    <div>
      <dt className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </dt>
      <dd className="mt-1 text-lg font-semibold tabular-nums">
        {value == null ? "No data" : `${value.toFixed(1)}%`}
      </dd>
    </div>
  );
}

function LatencySparkline({ points }: { points: HistoryPoint[] }) {
  const observed = points.filter(
    (point) => point.status !== "gap" && point.avgResponseTimeMs !== null,
  );
  if (observed.length < 2) {
    return (
      <div className="flex h-14 items-center justify-center rounded-md bg-muted/40 text-xs text-muted-foreground">
        Waiting for latency data
      </div>
    );
  }
  const values = observed.map((point) => point.avgResponseTimeMs as number);
  const min = Math.min(...values);
  const max = Math.max(...values);
  const spread = Math.max(max - min, 1);
  const segments: string[][] = [];
  let segment: string[] = [];
  points.forEach((point, index) => {
    if (point.status === "gap" || point.avgResponseTimeMs === null) {
      if (segment.length > 1) segments.push(segment);
      segment = [];
      return;
    }
    const x = points.length === 1 ? 0 : (index / (points.length - 1)) * 100;
    const y = 36 - ((point.avgResponseTimeMs - min) / spread) * 30;
    segment.push(`${x},${y}`);
  });
  if (segment.length > 1) segments.push(segment);

  return (
    <svg
      aria-label="24-hour latency sparkline"
      className="h-14 w-full rounded-md bg-muted/30"
      viewBox="0 0 100 42"
      preserveAspectRatio="none"
    >
      {segments.map((line, index) => (
        <polyline
          key={index}
          points={line.join(" ")}
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          vectorEffect="non-scaling-stroke"
          className="text-sky-500"
        />
      ))}
    </svg>
  );
}

function MonitorCard({ monitor }: { monitor: Monitor }) {
  const stats = useUptimeStats(monitor.id);
  const history = useHistory(monitor.id, "24h");

  return (
    <Link
      to={`/monitors/${monitor.id}`}
      className="group rounded-xl focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      data-testid="monitor-card"
    >
      <Card className="h-full gap-4 transition-colors group-hover:border-foreground/20">
        <CardHeader className="gap-3">
          <div className="flex items-center justify-between gap-3">
            <StatusDot status={monitor.uptimeStatus} />
            <ArrowRight className="size-4 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
          </div>
          <p className="truncate text-sm font-semibold" title={monitor.url}>
            {monitor.url}
          </p>
        </CardHeader>
        <CardContent className="space-y-4">
          {stats.isError ? (
            <p className="text-sm text-destructive">Could not load uptime stats.</p>
          ) : (
            <dl className="grid grid-cols-3 gap-3">
              <Stat label="24 hours" value={stats.data?.uptime24h} />
              <Stat label="7 days" value={stats.data?.uptime7d} />
              <Stat label="30 days" value={stats.data?.uptime30d} />
            </dl>
          )}
          <LatencySparkline points={history.data?.points ?? []} />
          <div className="flex items-center justify-between text-xs text-muted-foreground">
            <span>Last response {formatMs(monitor.lastResponseTimeMs)}</span>
            <span>{formatTimeAgo(monitor.lastCheckAt)}</span>
          </div>
        </CardContent>
      </Card>
    </Link>
  );
}

export function DashboardPage() {
  const monitors = useMonitors();
  const sorted = [...(monitors.data ?? [])].sort(
    (a, b) => STATUS_ORDER[a.uptimeStatus] - STATUS_ORDER[b.uptimeStatus] || a.url.localeCompare(b.url),
  );

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold tracking-tight">Dashboard</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Current health and measured uptime across every monitor.
        </p>
      </div>

      {monitors.isPending ? (
        <p className="text-sm text-muted-foreground">Loading monitors…</p>
      ) : monitors.isError ? (
        <p className="text-sm text-destructive">Could not load monitors.</p>
      ) : sorted.length === 0 ? (
        <div className="flex flex-col items-center gap-4 rounded-xl border border-dashed py-16 text-center">
          <Activity className="size-8 text-muted-foreground" />
          <div>
            <p className="font-medium">No monitors yet</p>
            <p className="mt-1 text-sm text-muted-foreground">
              Add a URL to start collecting uptime and latency history.
            </p>
          </div>
          <Link className="text-sm font-medium underline underline-offset-4" to="/monitors">
            Add your first monitor
          </Link>
        </div>
      ) : (
        <div className="grid gap-4 lg:grid-cols-2 xl:grid-cols-3">
          {sorted.map((monitor) => (
            <MonitorCard key={monitor.id} monitor={monitor} />
          ))}
        </div>
      )}
    </div>
  );
}
