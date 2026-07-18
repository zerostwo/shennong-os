"use client";

import Link from "next/link";
import Image from "next/image";
import { usePathname, useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Activity,
  ArrowLeft,
  Beaker,
  Bot,
  Boxes,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  CircleHelp,
  BookOpen,
  Database,
  FolderKanban,
  KeyRound,
  LayoutDashboard,
  ListFilter,
  LogIn,
  LogOut,
  Menu,
  MessageSquare,
  MoreHorizontal,
  PanelLeft,
  Plus,
  Puzzle,
  Search,
  Server,
  Settings,
  ShieldCheck,
  SquarePen,
  TicketCheck,
  UserPlus,
  UserRound,
  X,
} from "lucide-react";
import {
  getSession,
  getPublicConfig,
  searchWorkspace,
  signOut,
  type WorkspaceSearchItem,
} from "@/lib/api/adapter";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { SettingsDialog } from "@/components/settings-dialog";
import { settingsHash, settingsSectionFromHash, type SettingsSection } from "@/lib/settings-route";
import { ShennongThreadList } from "@/components/assistant-ui/thread-list";
import { ProfileDialog } from "@/components/profile-dialog";
import type { AuthSession } from "@/lib/auth-session";

type ShellProps = {
  active: string;
  variant?: "public" | "admin";
  assistantThreads?: boolean;
  children: React.ReactNode;
};

type SessionRecord = AuthSession;

const adminGroups = [
  { label: "OPERATIONS", items: [
    ["Overview", "/admin/dashboard", LayoutDashboard],
    ["Users", "/admin/users", UserRound],
    ["Invitations", "/admin/invites", TicketCheck],
  ] },
  { label: "INFRASTRUCTURE", items: [
    ["Model providers", "/admin/models", Bot],
    ["Resource providers", "/admin/providers", Database],
    ["System health", "/admin/monitoring", Server],
  ] },
  { label: "GOVERNANCE", items: [
    ["Audit events", "/admin/audit", Activity],
    ["Registration", "/admin/security", ShieldCheck],
  ] },
] as const;

export function AppShell({ variant = "public", assistantThreads = false, children }: ShellProps) {
  const [mobileOpen, setMobileOpen] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const [profileOpen, setProfileOpen] = useState(false);
  const [editProfileOpen, setEditProfileOpen] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsSection, setSettingsSection] = useState<SettingsSection>("general");
  const [session, setSession] = useState<SessionRecord | null>(null);
  const [registrationOpen, setRegistrationOpen] = useState(false);
  const pathname = usePathname();
  const isAdmin = variant === "admin";

  const openSettingsRoute = useCallback((section: SettingsSection, mode: "push" | "replace" = "push") => {
    setSettingsSection(section);
    setSettingsOpen(true);
    const hash = settingsHash(section);
    if (window.location.hash === hash) return;
    window.history[mode === "push" ? "pushState" : "replaceState"](null, "", `${window.location.pathname}${window.location.search}${hash}`);
  }, []);

  const changeSettingsOpen = useCallback((open: boolean) => {
    setSettingsOpen(open);
    if (!open && settingsSectionFromHash(window.location.hash)) {
      window.history.replaceState(null, "", `${window.location.pathname}${window.location.search}`);
    }
  }, []);

  useEffect(() => {
    const savedDensity = window.localStorage.getItem("shennong.interface-density");
    document.documentElement.dataset.density = savedDensity === "compact" ? "compact" : "comfortable";
    void getSession()
      .then((value) => setSession(value))
      .catch(() => setSession({ authenticated: false, user_id: "", role: "", scopes: [] }));
    void getPublicConfig()
      .then((value) => setRegistrationOpen(value.registration_mode === "open" || value.registration_mode === "invite_only" || value.registration_enabled === true))
      .catch(() => setRegistrationOpen(false));
  }, []);

  useEffect(() => {
    const syncSettingsRoute = () => {
      const section = settingsSectionFromHash(window.location.hash);
      setSettingsOpen(Boolean(section));
      if (section) setSettingsSection(section);
    };
    syncSettingsRoute();
    window.addEventListener("hashchange", syncSettingsRoute);
    window.addEventListener("popstate", syncSettingsRoute);
    return () => {
      window.removeEventListener("hashchange", syncSettingsRoute);
      window.removeEventListener("popstate", syncSettingsRoute);
    };
  }, []);

  useEffect(() => {
    const shortcut = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setSearchOpen(true);
      }
    };
    const openSettings = (event: Event) => {
      const value = (event as CustomEvent<string>).detail;
      openSettingsRoute(value === "models" || value === "memory" || value === "account" || value === "personalization" || value === "keyboard" ? value : "general");
    };
    const openProfile = () => { changeSettingsOpen(false); setEditProfileOpen(true); };
    window.addEventListener("keydown", shortcut);
    window.addEventListener("shennong:open-settings", openSettings);
    window.addEventListener("shennong:open-profile", openProfile);
    return () => {
      window.removeEventListener("keydown", shortcut);
      window.removeEventListener("shennong:open-settings", openSettings);
      window.removeEventListener("shennong:open-profile", openProfile);
    };
  }, [changeSettingsOpen, openSettingsRoute]);

  useEffect(() => { setMobileOpen(false); setProfileOpen(false); }, [pathname]);

  return (
    <div className={`app-shell shennong-shell ${isAdmin ? "admin-shell" : "public-shell"} ${collapsed ? "sidebar-collapsed" : ""}`}>
      <button className="mobile-menu-button" onClick={() => setMobileOpen(true)} aria-label="Open navigation"><Menu /></button>
      {mobileOpen ? <button className="sidebar-mobile-scrim" onClick={() => setMobileOpen(false)} aria-label="Close navigation" /> : null}
      <aside className={`sidebar shennong-sidebar ${mobileOpen ? "sidebar-open" : ""}`}>
        <div className="sidebar-topbar">
          <Link href={isAdmin ? "/admin/dashboard" : "/"} className="brand" aria-label="Shennong home"><span className="brand-symbol"><Beaker /></span><span>Shennong</span></Link>
          <button className="icon-button collapse-button" onClick={() => setCollapsed((value) => !value)} aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}>{collapsed ? <ChevronRight /> : <ChevronLeft />}</button>
          <button className="icon-button sidebar-close" onClick={() => setMobileOpen(false)} aria-label="Close navigation"><X /></button>
        </div>
        {isAdmin ? <AdminNav pathname={pathname} /> : <PublicNav pathname={pathname} authenticated={Boolean(session?.authenticated)} assistantThreads={assistantThreads} openSearch={() => setSearchOpen(true)} />}
        <div className="sidebar-spacer" />
        {isAdmin ? <AdminFooter session={session} /> : <PublicFooter session={session} registrationOpen={registrationOpen} profileOpen={profileOpen} onProfile={() => setProfileOpen((value) => !value)} openProfile={() => { setProfileOpen(false); setEditProfileOpen(true); }} openSettings={(section) => { openSettingsRoute(section); setProfileOpen(false); }} />}
      </aside>
      <main className="main-column">{children}</main>
      <WorkspaceSearchDialog open={searchOpen} onOpenChange={setSearchOpen} />
      <SettingsDialog open={settingsOpen} onOpenChange={changeSettingsOpen} onSectionChange={(section) => openSettingsRoute(section, "replace")} session={session} initialSection={settingsSection} />
      <ProfileDialog open={editProfileOpen} onOpenChange={setEditProfileOpen} session={session} onSaved={setSession} />
    </div>
  );
}

function PublicNav({ pathname, authenticated, assistantThreads, openSearch }: { pathname: string; authenticated: boolean; assistantThreads: boolean; openSearch: () => void }) {
  return (
    <nav className="sidebar-nav public-primary-nav" aria-label="Main navigation">
      {assistantThreads ? <ShennongThreadList /> : <NavItem label="New chat" href="/" icon={SquarePen} active={pathname === "/"} />}
      <button className="nav-item sidebar-command" onClick={openSearch}><Search /><span>Search</span><kbd>⌘K</kbd></button>
      <NavItem label="Resources" href="/resources" icon={Database} active={pathname === "/resources" || pathname.startsWith("/resources/") || pathname.startsWith("/catalog")} />
      {authenticated ? <NavItem label="Projects" href="/projects" icon={FolderKanban} active={pathname.startsWith("/projects")} /> : null}
      <NavItem label="Plugins" href="/plugins" icon={Puzzle} active={pathname.startsWith("/plugins")} />
      <NavItem label="Docs" href="/docs" icon={BookOpen} active={pathname.startsWith("/docs")} />
    </nav>
  );
}

function AdminNav({ pathname }: { pathname: string }) {
  return (
    <nav className="sidebar-nav admin-nav" aria-label="Administrator navigation">
      {adminGroups.map((group) => <div className="admin-nav-group" key={group.label}><div className="nav-label">{group.label}</div>{group.items.map(([label, href, Icon]) => <NavItem key={label} label={label} href={href} icon={Icon} active={pathname === href || pathname.startsWith(`${href}/`)} />)}</div>)}
    </nav>
  );
}

function NavItem({ label, href, icon: Icon, active }: { label: string; href: string; icon: typeof LayoutDashboard; active: boolean }) {
  return <Link href={href} className={`nav-item ${active ? "active" : ""}`} title={label}><Icon /><span>{label}</span></Link>;
}

function PublicFooter({ session, registrationOpen, profileOpen, onProfile, openProfile, openSettings }: { session: SessionRecord | null; registrationOpen: boolean; profileOpen: boolean; onProfile: () => void; openProfile: () => void; openSettings: (section: SettingsSection) => void }) {
  if (!session?.authenticated) return (
    <div className="sidebar-footer signed-out-footer">
      <Link className="sidebar-auth-primary" href="/auth/sign-in"><LogIn />Sign in</Link>
      {registrationOpen ? <Link className="sidebar-auth-secondary" href="/auth/sign-in?mode=register"><UserPlus />Create account</Link> : null}
    </div>
  );
  return (
    <div className="sidebar-footer">
      <div className="profile-popover-wrap">
        {profileOpen ? (
          <div className="profile-popover" role="menu">
            <button onClick={openProfile}><UserRound />Edit profile</button>
            <button onClick={() => openSettings("general")}><Settings />Settings</button>
            <Link href="/console/jobs"><Activity />Analysis jobs</Link>
            <Link href="/console/sessions"><UserRound />Active sessions</Link>
            {session.role === "admin" ? <Link href="/admin/dashboard" className="admin-link"><ShieldCheck />Admin center</Link> : null}
            <Link href="/support"><CircleHelp />Help</Link>
            <button className="danger-menu" onClick={() => void signOut().then(() => location.assign("/"))}><LogOut />Sign out</button>
          </div>
        ) : null}
        <button className="profile-button" onClick={onProfile} aria-expanded={profileOpen}>
          <ProfileAvatar session={session} />
          <span className="profile-copy"><strong>{session.display_name || session.username || session.user_id}</strong><small>{session.username ? `@${session.username}` : session.role}</small></span>
          <ChevronDown />
        </button>
      </div>
    </div>
  );
}

function ProfileAvatar({ session }: { session: SessionRecord }) {
  return session.avatar_url
    ? <span className="avatar avatar-image"><Image src={session.avatar_url} alt="" width={28} height={28} unoptimized /></span>
    : <span className="avatar avatar-green">{(session.display_name || session.username || session.user_id).slice(0, 1).toUpperCase()}</span>;
}

function AdminFooter({ session }: { session: SessionRecord | null }) {
  return (
    <div className="admin-footer">
      <Link href="/" className="return-portal"><ArrowLeft />Return to Agent Chat</Link>
      {session?.authenticated ? <div className="admin-user"><span className="avatar avatar-dark">{session.user_id.slice(0, 1).toUpperCase()}</span><span><strong>{session.user_id}</strong><small>{session.role}</small></span></div> : <Link className="primary-button sign-in-button" href="/auth/sign-in"><KeyRound />Sign in</Link>}
    </div>
  );
}

function WorkspaceSearchDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (value: boolean) => void }) {
  const router = useRouter();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<WorkspaceSearchItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  useEffect(() => {
    if (!open) { setQuery(""); setResults([]); setError(""); return; }
    if (!query.trim()) { setResults([]); setLoading(false); return; }
    let cancelled = false;
    const timer = window.setTimeout(() => {
      setLoading(true);
      setError("");
      void searchWorkspace(query)
        .then((value) => { if (!cancelled) { setResults(value); setActiveIndex(0); } })
        .catch((reason) => { if (!cancelled) setError(reason instanceof Error ? reason.message : "Search failed"); })
        .finally(() => { if (!cancelled) setLoading(false); });
    }, 180);
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [open, query]);
  const grouped = useMemo(() => ({
    chat: results.filter((item) => item.kind === "chat"),
    resource: results.filter((item) => item.kind === "resource"),
    project: results.filter((item) => item.kind === "project"),
  }), [results]);
  function navigate(item: WorkspaceSearchItem) { onOpenChange(false); router.push(item.href); }
  function handleKey(event: React.KeyboardEvent<HTMLInputElement>) {
    if (event.key === "ArrowDown") { event.preventDefault(); setActiveIndex((value) => Math.min(results.length - 1, value + 1)); }
    if (event.key === "ArrowUp") { event.preventDefault(); setActiveIndex((value) => Math.max(0, value - 1)); }
    if (event.key === "Enter" && results[activeIndex]) { event.preventDefault(); navigate(results[activeIndex]); }
  }
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="workspace-search-dialog" showCloseButton={false}>
        <DialogTitle className="sr-only">Search Shennong</DialogTitle>
        <DialogDescription className="sr-only">Search chats, Resources, and Projects.</DialogDescription>
        <div className="workspace-search-input"><Search /><input autoFocus value={query} onChange={(event) => setQuery(event.target.value)} onKeyDown={handleKey} placeholder="Search chats, Resources, and Projects" /><kbd>ESC</kbd></div>
        <div className="workspace-search-results">
          {!query.trim() ? <div className="search-quick-links"><Link href="/" onClick={() => onOpenChange(false)}><SquarePen /><span><strong>New chat</strong><small>Start a new agent conversation</small></span></Link><Link href="/resources" onClick={() => onOpenChange(false)}><Database /><span><strong>Resources</strong><small>Browse governed biomedical data</small></span></Link><Link href="/projects" onClick={() => onOpenChange(false)}><FolderKanban /><span><strong>Projects</strong><small>Open a research workspace</small></span></Link></div> : null}
          {loading ? <div className="search-state">Searching…</div> : null}
          {error ? <div className="search-state error" role="alert">{error}</div> : null}
          {!loading && !error && query.trim() && results.length === 0 ? <div className="search-state">No matching data</div> : null}
          {(["chat", "resource", "project"] as const).map((kind) => grouped[kind].length ? <div className="search-result-group" key={kind}><strong>{kind === "chat" ? "Chats" : kind === "resource" ? "Resources" : "Projects"}</strong>{grouped[kind].map((item) => { const index = results.indexOf(item); const Icon = kind === "chat" ? MessageSquare : kind === "resource" ? Database : FolderKanban; return <button key={`${kind}-${item.id}`} className={activeIndex === index ? "active" : ""} onMouseEnter={() => setActiveIndex(index)} onClick={() => navigate(item)}><Icon /><span><b>{item.title}</b>{item.description ? <small>{item.description}</small> : null}</span></button>; })}</div> : null)}
        </div>
        <div className="workspace-search-footer"><span><kbd>↑</kbd><kbd>↓</kbd> Navigate</span><span><kbd>↵</kbd> Open</span></div>
      </DialogContent>
    </Dialog>
  );
}

export function TopBar({ title, description, search: _search = true, action }: { title?: string; description?: string; search?: boolean; action?: React.ReactNode }) {
  void _search;
  if (!title && !action) return null;
  return (
    <header className="topbar">
      <div className="topbar-title">{title ? <><h1>{title}</h1>{description ? <p>{description}</p> : null}</> : null}</div>
      <div className="topbar-actions">{action}</div>
    </header>
  );
}

export function SectionHeader({ title, description, action }: { title: string; description?: string; action?: React.ReactNode }) {
  return <div className="section-header"><div><h2>{title}</h2>{description ? <p>{description}</p> : null}</div>{action}</div>;
}

export function IconButton({ children, label, onClick }: { children: React.ReactNode; label: string; onClick?: () => void }) {
  return <button className="icon-button" onClick={onClick} aria-label={label}>{children}</button>;
}

export function TinyBadge({ children, tone = "neutral" }: { children: React.ReactNode; tone?: "blue" | "green" | "amber" | "purple" | "neutral" }) {
  return <span className={`tiny-badge badge-${tone}`}>{children}</span>;
}

export function CopyButton({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);
  return <button className="copy-button" onClick={() => { void navigator.clipboard?.writeText(value); setCopied(true); window.setTimeout(() => setCopied(false), 1500); }} aria-label="Copy value">{copied ? "Copied" : "Copy"}</button>;
}

export function EmptyState({ title, description, action }: { title: string; description: string; action?: React.ReactNode }) {
  return <div className="empty-state"><div className="empty-icon"><Boxes /></div><h3>{title}</h3><p>{description}</p>{action}</div>;
}

export { MoreHorizontal, ListFilter, PanelLeft, Plus };
