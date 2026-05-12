import { NavLink, Outlet } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { FileText, Network, MessageSquare, Settings } from "lucide-react";
import { getConfig } from "@/lib/config";
import { cn } from "@/lib/utils";
import { LocaleSwitcher } from "@/components/LocaleSwitcher";
import { TokenDialog } from "@/components/TokenDialog";

const NAV_ITEMS = [
  { to: "/pages", key: "nav.pages", icon: FileText },
  { to: "/graph", key: "nav.graph", icon: Network },
  { to: "/query", key: "nav.query", icon: MessageSquare },
  { to: "/manifest", key: "nav.manifest", icon: Settings },
] as const;

export function Layout() {
  const { t } = useTranslation();
  const cfg = getConfig();
  return (
    <div className="min-h-screen flex flex-col bg-background text-foreground">
      <header className="border-b sticky top-0 z-40 bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
        <div className="container flex h-14 items-center gap-6">
          <div className="flex items-center gap-2 font-semibold">
            <div className="h-7 w-7 rounded-full bg-primary text-primary-foreground grid place-items-center text-xs">
              C
            </div>
            <span>{t("app.title")}</span>
            <span className="text-xs text-muted-foreground hidden md:inline">
              {t("app.tagline")}
            </span>
          </div>
          <nav className="flex items-center gap-1">
            {NAV_ITEMS.map(({ to, key, icon: Icon }) => (
              <NavLink
                key={to}
                to={to}
                className={({ isActive }) =>
                  cn(
                    "inline-flex items-center gap-2 rounded-md px-3 py-1.5 text-sm transition-colors",
                    isActive
                      ? "bg-primary text-primary-foreground"
                      : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                  )
                }
              >
                <Icon className="h-4 w-4" />
                {t(key)}
              </NavLink>
            ))}
          </nav>
          <div className="ml-auto flex items-center gap-2">
            {cfg.authRequired ? <TokenDialog /> : null}
            <LocaleSwitcher />
          </div>
        </div>
      </header>
      <main className="flex-1 container py-6">
        <Outlet />
      </main>
      <footer className="border-t py-4 text-xs text-muted-foreground">
        <div className="container">
          {t("footer.version", { version: cfg.version })}
        </div>
      </footer>
    </div>
  );
}
