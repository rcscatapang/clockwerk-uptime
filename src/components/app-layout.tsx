import type { ReactNode } from "react";

export function AppLayout({ children }: { children: ReactNode }) {
  return (
    <div className="flex min-h-screen flex-col">
      <header className="flex h-12 shrink-0 items-center border-b px-4">
        <span className="text-sm font-semibold">Uptime Monitor</span>
      </header>
      <main className="flex-1 p-6">{children}</main>
    </div>
  );
}
