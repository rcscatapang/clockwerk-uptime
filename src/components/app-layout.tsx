import { NavLink, Outlet } from "react-router";
import { Activity, LayoutDashboard, Settings } from "lucide-react";
import packageJson from "../../package.json";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { ClockwerkLockup } from "@/components/clockwerk-wordmark";
import { cn } from "@/lib/utils";
import markUrl from "@/assets/clockwerk-mark.png";

const NAV_ITEMS = [
  { to: "/", label: "Dashboard", icon: LayoutDashboard, end: true },
  { to: "/monitors", label: "Monitors", icon: Activity, end: false },
];

const navLinkClassName = ({ isActive }: { isActive: boolean }) =>
  cn(
    "flex items-center gap-2 rounded-md px-3 py-2 text-sm font-medium transition-colors",
    isActive
      ? "bg-accent text-accent-foreground"
      : "text-muted-foreground hover:bg-accent/50 hover:text-foreground",
  );

export function AppLayout() {
  return (
    <div className="flex h-screen">
      <aside className="flex w-48 shrink-0 flex-col border-r bg-muted/30">
        <nav aria-label="Primary navigation" className="flex flex-col gap-1 p-2">
          {NAV_ITEMS.map(({ to, label, icon: Icon, end }) => (
            <NavLink
              key={to}
              to={to}
              end={end}
              className={navLinkClassName}
            >
              <Icon className="size-4" />
              {label}
            </NavLink>
          ))}
        </nav>
        <div className="mt-auto">
          <nav aria-label="Utility navigation" className="p-2">
            <NavLink to="/settings" className={navLinkClassName}>
              <Settings className="size-4" />
              Settings
            </NavLink>
          </nav>
          <Dialog>
            <DialogTrigger asChild>
              <button
                type="button"
                className="group flex w-full items-center gap-3 border-t px-4 py-3 text-left transition-colors hover:bg-accent/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
                aria-label="About Clockwerk"
              >
                <img
                  src={markUrl}
                  alt=""
                  className="size-7 shrink-0 transition-transform group-hover:scale-105"
                />
                <span className="min-w-0">
                  <span className="block text-sm font-semibold leading-tight">Clockwerk</span>
                  <span className="block text-[11px] leading-tight text-muted-foreground">
                    v{packageJson.version}
                  </span>
                </span>
              </button>
            </DialogTrigger>
            <DialogContent className="sm:max-w-sm">
              <DialogHeader className="items-center pt-2 text-center sm:text-center">
                <DialogTitle className="pb-1">
                  <ClockwerkLockup />
                </DialogTitle>
                <DialogDescription>
                  Uptime and SSL monitoring, quietly running in your menu bar.
                </DialogDescription>
              </DialogHeader>
              <div className="rounded-md border bg-muted/30 px-3 py-2.5 text-sm">
                <div className="flex items-center justify-between gap-4">
                  <span className="text-muted-foreground">Version</span>
                  <span className="font-mono text-xs">{packageJson.version}</span>
                </div>
              </div>
            </DialogContent>
          </Dialog>
        </div>
      </aside>
      <main className="flex-1 overflow-y-auto p-6">
        <Outlet />
      </main>
    </div>
  );
}
