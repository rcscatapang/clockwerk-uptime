import { NavLink, Outlet } from "react-router";
import { Activity, LayoutDashboard, Settings } from "lucide-react";

import { cn } from "@/lib/utils";

const NAV_ITEMS = [
  { to: "/", label: "Dashboard", icon: LayoutDashboard, end: true },
  { to: "/monitors", label: "Monitors", icon: Activity, end: false },
  { to: "/settings", label: "Settings", icon: Settings, end: false },
];

export function AppLayout() {
  return (
    <div className="flex h-screen">
      <aside className="flex w-48 shrink-0 flex-col border-r bg-muted/30">
        <div className="flex h-12 items-center border-b px-4">
          <span className="text-sm font-semibold">Uptime Monitor</span>
        </div>
        <nav className="flex flex-col gap-1 p-2">
          {NAV_ITEMS.map(({ to, label, icon: Icon, end }) => (
            <NavLink
              key={to}
              to={to}
              end={end}
              className={({ isActive }) =>
                cn(
                  "flex items-center gap-2 rounded-md px-3 py-2 text-sm font-medium transition-colors",
                  isActive
                    ? "bg-accent text-accent-foreground"
                    : "text-muted-foreground hover:bg-accent/50 hover:text-foreground",
                )
              }
            >
              <Icon className="size-4" />
              {label}
            </NavLink>
          ))}
        </nav>
      </aside>
      <main className="flex-1 overflow-y-auto p-6">
        <Outlet />
      </main>
    </div>
  );
}
