import { useNavigate } from "react-router";

import { Button } from "@/components/ui/button";

// Placeholder until history and stats land; the monitors page is the working
// surface for now.
export function DashboardPage() {
  const navigate = useNavigate();
  return (
    <div className="mx-auto flex max-w-md flex-col items-center gap-4 pt-16 text-center">
      <h1 className="text-2xl font-bold tracking-tight">Dashboard</h1>
      <p className="text-sm text-muted-foreground">
        Uptime stats and charts will appear here once monitors have collected
        some history.
      </p>
      <Button variant="outline" onClick={() => navigate("/monitors")}>
        Manage monitors
      </Button>
    </div>
  );
}
