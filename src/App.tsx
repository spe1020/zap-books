import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowDownToLine,
  Download,
  ArrowLeftRight,
  ArrowRight,
  ArrowUpRight,
  Building2,
  CalendarDays,
  ChartNoAxesCombined,
  Check,
  CheckCheck,
  ChevronDown,
  CircleHelp,
  FileText,
  Inbox,
  KeyRound,
  LayoutDashboard,
  LogOut,
  Menu,
  Plus,
  Search,
  Settings2,
  ShieldCheck,
  Sparkles,
  Users,
  X,
} from "lucide-react";
import type {
  BusinessData,
  Currency,
  ExpenseDraft,
  JournalEntry,
  Workspace,
} from "./domain/types";
import { fiatCurrencies } from "./domain/types";
import {
  blankDraft,
  categoryTotals,
  currentPeriod,
  duplicates,
  missingDetails,
  money,
  today,
  totals,
} from "./domain/ledger";
import { localLedger } from "./adapters/local-ledger";
import type { LedgerClient } from "./adapters/ledger-client";
import type { RemoteLedgerClient, RemoteView } from "./remote";
import { SyncStatus } from "./components/SyncStatus";
import {
  Members,
  RecoveryBanner,
  type PeopleTools,
} from "./components/Members";
import { transport } from "./adapters/transport";
import { demoAI } from "./adapters/ai";
import { BRAND, PRODUCT_NAME } from "./config";
import {
  ArrowLink,
  Empty,
  Modal,
  SaveErrorContext,
  shortDate,
  Status,
  VendorIcon,
} from "./components/common";
import { Review } from "./components/Review";
import type { Mutation } from "./components/Review";
import { ImportDialog } from "./components/Import";
import { ExportDialog } from "./components/Export";
import { Settings } from "./components/Settings";
import { RecordTransfer } from "./components/Transfer";
import {
  accountKindLabels,
  balanceMeanings,
  balances,
  transferTypeLabels,
} from "./domain/transfers";
import { ErrorBoundary } from "./components/ErrorBoundary";
type View = "overview" | "inbox" | "transactions" | "settings" | "members";
const nav = [
  { id: "overview", label: "Overview", icon: LayoutDashboard },
  { id: "inbox", label: "Expense inbox", icon: Inbox },
  { id: "transactions", label: "Transactions", icon: ArrowLeftRight },
  { id: "settings", label: "Settings", icon: Settings2 },
] as const;
/** Books on the service have members. */
const remoteNav = [
  ...nav,
  { id: "members", label: "Members", icon: Users },
] as const;
/** How often to sync while the window has focus. */
const POLL_MS = 15_000;
/**
 * The workspace. With `remote`, its books are on the ledger service;
 * otherwise they are the local demo's.
 */
export default function App({
  remote,
  remoteNpub,
  peopleTools,
  onSignOut,
  onNewBusiness,
  onRecoveryMode,
}: {
  remote?: RemoteLedgerClient;
  remoteNpub?: string;
  peopleTools?: (business: string) => PeopleTools;
  onSignOut?: () => void;
  onNewBusiness?: () => void;
  /** For a key that is also the backup key of some business. */
  onRecoveryMode?: () => void;
} = {}) {
  const client: LedgerClient = remote ?? localLedger;
  const [workspace, setWorkspace] = useState<Workspace>();
  const [remoteView, setRemoteView] = useState<RemoteView | undefined>(
    remote?.view(),
  );
  const [businessId, setBusinessId] = useState(() => {
    try {
      return localStorage.getItem("business-finance-active") || "northstar";
    } catch {
      return "northstar";
    }
  });
  const [view, setView] = useState<View>("overview");
  const [period, setPeriod] = useState(currentPeriod());
  const [currency, setCurrency] = useState<Currency>("USD");
  const [error, setError] = useState(""),
    [toast, setToast] = useState("");
  const [review, setReview] = useState<ExpenseDraft | null>(null);
  const [importing, setImporting] = useState(false),
    [exporting, setExporting] = useState(false),
    [assistant, setAssistant] = useState(false),
    [mobileNav, setMobileNav] = useState(false);
  const [answer, setAnswer] = useState("");
  const [reset, setReset] = useState(false),
    [resetText, setResetText] = useState("");
  const [transfer, setTransfer] = useState<JournalEntry | null>(null);
  const [recording, setRecording] = useState(false);
  const [correcting, setCorrecting] = useState<JournalEntry | null>(null);
  const [busy, setBusy] = useState(false);
  const lock = useRef(false);
  useEffect(() => {
    if (!remote) return;
    const update = () => {
      setWorkspace(remote.workspace());
      setRemoteView(remote.view());
    };
    const unsubscribe = remote.subscribe(update);
    const poll = () => {
      if (document.hasFocus()) void remote.refresh();
    };
    const timer = setInterval(poll, POLL_MS);
    const onFocus = () => void remote.refresh();
    window.addEventListener("focus", onFocus);
    return () => {
      unsubscribe();
      clearInterval(timer);
      window.removeEventListener("focus", onFocus);
    };
  }, [remote]);
  useEffect(() => {
    let active = true;
    client
      .load()
      .then((w) => {
        if (active) setWorkspace(w);
      })
      .catch((e) => {
        if (active) setError(e.message);
      });
    return () => {
      active = false;
    };
  }, [client]);
  useEffect(() => {
    if (toast) {
      const t = setTimeout(() => setToast(""), 5000);
      return () => clearTimeout(t);
    }
  }, [toast]);
  const b =
    workspace?.businesses.find((b) => b.profile.id === businessId) ??
    workspace?.businesses[0];
  const mutate: Mutation = useCallback(
    async (commands, message) => {
      if (!workspace || !b || lock.current) return false;
      lock.current = true;
      setBusy(true);
      setError("");
      try {
        const saved = await client.run(b.profile.id, commands);
        setWorkspace(saved);
        const next = saved.businesses.find(
          (x) => x.profile.id === b.profile.id,
        );
        await transport.publish(next?.events ?? []);
        setToast(message);
        return true;
      } catch (e) {
        // Signed and queued: it goes when the service can be reached.
        if ((e as Error).name === "Waiting") {
          setToast((e as Error).message);
          return true;
        }
        setError((e as Error).message);
        return false;
      } finally {
        lock.current = false;
        setBusy(false);
      }
    },
    [b, workspace, client],
  );
  function switchBusiness(next: string) {
    setBusinessId(next);
    try {
      localStorage.setItem("business-finance-active", next);
    } catch {
      /* Preference persistence is optional; records use IndexedDB. */
    }
    setReview(null);
    setImporting(false);
    setExporting(false);
    setAssistant(false);
    setAnswer("");
    setError("");
    setMobileNav(false);
    setCurrency(
      workspace?.businesses.find((x) => x.profile.id === next)?.profile
        .currency ?? "USD",
    );
  }
  function navigate(next: View) {
    setView(next);
    setMobileNav(false);
  }
  if (!workspace || !b)
    return (
      <main className="loading">
        <h1 className="brand-heading">
          <img className="brand-logo" src={BRAND.logo} alt={PRODUCT_NAME} />
        </h1>
        {error ? (
          <>
            <p role="alert">{error}</p>
            <button
              className="button primary"
              onClick={() => location.reload()}
            >
              Retry loading workspace
            </button>
          </>
        ) : (
          <p>Opening your workspace…</p>
        )}
      </main>
    );
  const drafts = b.drafts.filter((d) => d.status === "draft");
  const navItems = remote ? remoteNav : nav;
  // Books on the service: what the signed-in key's role allows. The service
  // decides; this only hides what it would refuse.
  const membership = remote?.membership(b.profile.id);
  const people = remote?.people(b.profile.id);
  const me = remote?.signer.pubkey;
  const role = me ? membership?.members[me]?.role : undefined;
  const writes = !remote || role === "owner" || role === "bookkeeper";
  const canEdit = (d: ExpenseDraft) =>
    writes ||
    (role === "assistant" && membership?.assistantDrafts?.[d.id] === me);
  const tools = peopleTools?.(b.profile.id);
  return (
    <SaveErrorContext.Provider value={error}>
      <div className="app-shell">
        {mobileNav && (
          <button
            className="nav-backdrop"
            aria-label="Close navigation"
            onClick={() => setMobileNav(false)}
          />
        )}
        <aside className={`sidebar ${mobileNav ? "open" : ""}`}>
          <a
            className="brand"
            href="#main"
            onClick={() => navigate("overview")}
          >
            <img
              className="brand-logo"
              src={BRAND.logoOnDark}
              alt={PRODUCT_NAME}
            />
          </a>
          <div className="workspace-picker">
            <span
              className="business-avatar"
              style={{ background: b.profile.color }}
            >
              {b.profile.initials}
            </span>
            <div>
              <span className="switcher-label">WORKSPACE</span>
              <select
                aria-label="Switch business"
                value={b.profile.id}
                onChange={(e) => switchBusiness(e.target.value)}
              >
                {workspace.businesses.map((x) => (
                  <option key={x.profile.id} value={x.profile.id}>
                    {x.profile.name}
                  </option>
                ))}
              </select>
            </div>
            <ChevronDown size={15} />
          </div>
          <p className="nav-label">WORKSPACE</p>
          <nav aria-label="Main navigation">
            {navItems.map(({ id, label, icon: Icon }) => (
              <button
                key={id}
                onClick={() => navigate(id)}
                className={view === id ? "active" : ""}
                aria-current={view === id ? "page" : undefined}
              >
                <Icon size={19} strokeWidth={1.6} />
                <span>{label}</span>
                {id === "inbox" && (
                  <span className="nav-count">{drafts.length}</span>
                )}
              </button>
            ))}
          </nav>
          <div className="sidebar-bottom">
            <div className="sidebar-note">
              <span className="note-icon">
                <ShieldCheck size={20} />
              </span>
              <h3>A place for every detail.</h3>
              <p>Your expenses, evidence, and decisions. All together.</p>
            </div>
            {onRecoveryMode && (
              <button className="help-button" onClick={onRecoveryMode}>
                <KeyRound size={18} />
                Backup key mode
                <ArrowUpRight size={15} />
              </button>
            )}
            {remote && onNewBusiness && (
              <button className="help-button" onClick={onNewBusiness}>
                <Plus size={18} />
                New business
                <ArrowUpRight size={15} />
              </button>
            )}
            <button
              className="help-button"
              onClick={() => {
                setAnswer("");
                setAssistant(true);
              }}
            >
              <CircleHelp size={18} />
              Workspace guide
              <ArrowUpRight size={15} />
            </button>
            {remote ? (
              <div className="sidebar-user">
                <span className="user-avatar">
                  <KeyRound size={15} />
                </span>
                <div>
                  <strong title={remoteNpub}>
                    {remoteNpub?.slice(0, 12)}…{remoteNpub?.slice(-6)}
                  </strong>
                  <small>Signed in with {remote.signer.name}</small>
                </div>
                <button
                  className="icon-button"
                  aria-label="Sign out"
                  onClick={onSignOut}
                >
                  <LogOut size={15} />
                </button>
              </div>
            ) : (
              <div className="sidebar-user">
                <span className="user-avatar">JD</span>
                <div>
                  <strong>Jamie Davis</strong>
                  <small>Demo workspace owner</small>
                </div>
                <span className="online-dot" />
              </div>
            )}
          </div>
        </aside>
        <div className="main-shell">
          <header className="topbar">
            <div className="breadcrumb">
              <button
                className="icon-button mobile-menu"
                onClick={() => setMobileNav(true)}
                aria-label="Open navigation"
              >
                <Menu size={21} />
              </button>
              <img className="brand-icon topbar-icon" src={BRAND.icon} alt="" />
              <Building2 size={15} />
              <span>{b.profile.name}</span>
              <span className="slash">/</span>
              <strong>{navItems.find((n) => n.id === view)?.label}</strong>
            </div>
            <div className="topbar-actions">
              {remote && remoteView ? (
                <SyncStatus
                  view={remoteView}
                  onDiscard={(id) => void remote.discard(id)}
                  onRedo={(id) => void remote.redo(id)}
                />
              ) : (
                <span className="demo-indicator">
                  <span />
                  Demo data
                </span>
              )}
              <button
                className="assistant-button"
                onClick={() => {
                  setAnswer("");
                  setAssistant(true);
                }}
              >
                <Sparkles size={16} />
                Ask assistant
              </button>
            </div>
          </header>
          <main id="main" className="main-content">
            <div className="page-heading">
              <div>
                <p className="eyebrow">
                  {view === "overview"
                    ? "THE BIG PICTURE"
                    : view === "inbox"
                      ? "FROM EVIDENCE TO ENTRY"
                      : view === "transactions"
                        ? "YOUR BUSINESS RECORD"
                        : view === "members"
                          ? "THE PEOPLE BEHIND THE BOOKS"
                          : "MAKE IT YOURS"}
                </p>
                <h1>
                  {view === "overview"
                    ? "Your business, at a glance."
                    : view === "inbox"
                      ? "A clearer path to done."
                      : view === "transactions"
                        ? "Every entry. The full story."
                        : view === "members"
                          ? "Members"
                          : "Workspace settings"}
                </h1>
                <p>
                  {view === "overview"
                    ? "Know where things stand. Keep your next step simple."
                    : view === "inbox"
                      ? "Review the details, connect the evidence, and close the loop."
                      : view === "transactions"
                        ? "Posted expenses, transfers, and a history you can follow."
                        : view === "members"
                          ? `Who can read and change ${b.profile.name}'s books, and what's waiting to change.`
                          : `The details that keep ${b.profile.name} running smoothly.`}
                </p>
              </div>
              {view !== "settings" && view !== "members" && (
                <div className="heading-actions">
                  <button
                    className="button secondary"
                    onClick={() => setExporting(true)}
                  >
                    <Download size={17} />
                    Export
                  </button>
                  {writes && (
                    <>
                      {view === "transactions" && (
                        <button
                          className="button secondary"
                          onClick={() => setRecording(true)}
                        >
                          <ArrowLeftRight size={17} />
                          Record transfer
                        </button>
                      )}
                      <button
                        className="button secondary"
                        onClick={() => setImporting(true)}
                      >
                        <ArrowDownToLine size={17} />
                        Import CSV
                      </button>
                      <button
                        className="button primary"
                        onClick={() => setReview(blankDraft(b))}
                      >
                        <Plus size={18} />
                        New expense
                      </button>
                    </>
                  )}
                </div>
              )}
            </div>
            {!!workspace.quarantine?.length && (
              <div role="alert" className="alert error global-error">
                <span>
                  {workspace.quarantine.length === 1
                    ? "1 saved record was damaged and has been set aside."
                    : `${workspace.quarantine.length} saved records were damaged and have been set aside.`}{" "}
                  They stay in storage on this device, but no screen, total or
                  export uses them. Resetting the demo clears them.
                </span>
              </div>
            )}
            {people && membership && tools && (
              <RecoveryBanner
                people={people}
                membership={membership}
                tools={tools}
                mutate={mutate}
                busy={busy}
                onDismiss={() => void remote?.dismissRecovery(b.profile.id)}
              />
            )}
            {remote?.stoppedFor(b.profile.id) && (
              <div role="alert" className="alert error global-error">
                <span>{remote.stoppedFor(b.profile.id)}</span>
              </div>
            )}
            {error && error !== remote?.stoppedFor(b.profile.id) && (
              <div role="alert" className="alert error global-error">
                <span>{error}</span>
                <button
                  className="icon-button"
                  onClick={() => setError("")}
                  aria-label="Dismiss error"
                >
                  <X size={16} />
                </button>
              </div>
            )}
            <ErrorBoundary
              label="This screen"
              resetKey={`${view}:${b.profile.id}`}
            >
              {view === "overview" && (
                <Overview
                  b={b}
                  period={period}
                  setPeriod={setPeriod}
                  currency={currency}
                  setCurrency={setCurrency}
                  open={setReview}
                  navigate={navigate}
                />
              )}
              {view === "inbox" && (
                <InboxView
                  key={b.profile.id}
                  b={b}
                  open={setReview}
                  create={writes ? () => setReview(blankDraft(b)) : undefined}
                  byAssistant={people?.byAssistant}
                />
              )}
              {view === "transactions" && (
                <Transactions
                  key={b.profile.id}
                  b={b}
                  open={setReview}
                  openTransfer={setTransfer}
                />
              )}
              {view === "settings" && (
                <Settings
                  key={b.profile.id}
                  b={b}
                  mutate={mutate}
                  busy={busy}
                  onReset={
                    remote
                      ? undefined
                      : () => {
                          setResetText("");
                          setReset(true);
                        }
                  }
                />
              )}
              {view === "members" && membership && people && tools && (
                <Members
                  key={b.profile.id}
                  membership={membership}
                  people={people}
                  tools={tools}
                  mutate={mutate}
                  busy={busy}
                />
              )}
            </ErrorBoundary>
            <footer className="page-footer">
              <span>
                <span className="local-dot" />
                {remote
                  ? "Books from the ledger service, verified in this browser"
                  : "Saved on this device"}
              </span>
              <span>
                {PRODUCT_NAME} <span className="footer-separator">/</span> A
                little more clarity.
              </span>
            </footer>
          </main>
        </div>
        {review && (
          <ErrorBoundary
            label="This expense"
            resetKey={review.id}
            onClose={() => setReview(null)}
          >
            <Review
              key={review.id}
              b={b}
              initial={review}
              mutate={mutate}
              onClose={() => setReview(null)}
              onOpen={setReview}
              busy={busy}
              canEdit={canEdit(review)}
              canApprove={writes}
              byAssistant={!!people?.byAssistant[review.id]}
            />
          </ErrorBoundary>
        )}
        {importing && (
          <ImportDialog
            b={b}
            mutate={mutate}
            onClose={() => setImporting(false)}
            busy={busy}
          />
        )}
        {exporting && (
          <ExportDialog b={b} onClose={() => setExporting(false)} />
        )}
        {recording && (
          <RecordTransfer
            b={b}
            mutate={mutate}
            busy={busy}
            onClose={() => setRecording(false)}
          />
        )}
        {correcting && (
          <RecordTransfer
            key={correcting.id}
            b={b}
            mutate={mutate}
            busy={busy}
            correcting={correcting}
            onClose={() => setCorrecting(null)}
          />
        )}
        {toast && (
          <div className="toast" role="status">
            <Check size={17} />
            {toast}
            <button
              className="icon-button"
              aria-label="Dismiss notification"
              onClick={() => setToast("")}
            >
              <X size={15} />
            </button>
          </div>
        )}
        {assistant && (
          <Modal
            title="Your workspace assistant"
            onClose={() => setAssistant(false)}
          >
            <div className="assistant-body">
              <div className="assistant-orb">
                <Sparkles size={28} />
              </div>
              <span className="badge neutral">
                Deterministic demo · no live AI
              </span>
              <h3>A little help finding clarity.</h3>
              <p>
                Answers use {b.profile.name}’s saved records for{" "}
                <strong>{period}</strong>. Documents are not read or analyzed.
              </p>
              <div className="assistant-prompts">
                {[
                  "What needs review?",
                  "What did we spend?",
                  "Which category is largest?",
                ].map((q) => (
                  <button
                    key={q}
                    onClick={() => setAnswer(demoAI.answer(b, q, period))}
                  >
                    {q}
                    <ArrowUpRight size={16} />
                  </button>
                ))}
              </div>
              {answer && (
                <div className="assistant-answer" role="status">
                  <Sparkles size={16} />
                  <p>{answer}</p>
                </div>
              )}
              <div className="guide-note">
                <h4>Your expense workflow</h4>
                <p>
                  Create a draft or import a payment. Attach a receipt, review
                  the fields, then approve a valid expense. Find its entry in
                  Transactions and use “Correct expense” to preserve a linked
                  history.
                </p>
                <small>
                  Local demo storage is not production authentication. No
                  payments, financial data, or documents are sent externally.
                </small>
              </div>
            </div>
          </Modal>
        )}
        {reset && (
          <Modal
            title="Reset demonstration data?"
            onClose={() => setReset(false)}
          >
            <div className="modal-form">
              <p>
                This deletes all local edits, imported transactions, and
                attachments for <strong>both businesses</strong> and restores
                the fictional examples. This cannot be undone.
              </p>
              <label>
                Type RESET to continue
                <input
                  value={resetText}
                  onChange={(e) => setResetText(e.target.value)}
                />
              </label>
              <button
                className="button danger"
                disabled={resetText !== "RESET" || busy}
                onClick={async () => {
                  if (lock.current) return;
                  lock.current = true;
                  setBusy(true);
                  try {
                    const fresh = await localLedger.reset();
                    setWorkspace(fresh);
                    setReset(false);
                    setView("overview");
                    setToast("Both demo businesses have been reset.");
                  } catch (e) {
                    setError((e as Error).message);
                  } finally {
                    lock.current = false;
                    setBusy(false);
                  }
                }}
              >
                Reset both businesses
              </button>
            </div>
          </Modal>
        )}
        {transfer && (
          <Modal title="Account transfer" onClose={() => setTransfer(null)}>
            <div className="modal-form">
              <span className="badge blue">
                {transfer.transferType
                  ? transferTypeLabels[transfer.transferType]
                  : "Transfer"}{" "}
                · excluded from spending
              </span>
              <h3>{transfer.memo}</h3>
              <p>
                {transfer.date}
                {transfer.purpose && ` · ${transfer.purpose}`}
                {transfer.reference && ` · Reference ${transfer.reference}`}
              </p>
              {b.journals.some((j) => j.reversesId === transfer.id) && (
                <p className="small muted">
                  Corrected: a reversal and a replacement were posted.
                </p>
              )}
              {transfer.lines.map((l, i) => (
                <div className="setting-row" key={i}>
                  <strong>
                    {b.accounts.find((a) => a.id === l.accountId)?.name}
                  </strong>
                  <span>
                    {l.debit ? "To" : "From"} ·{" "}
                    {money(l.debit || l.credit, transfer.currency)}
                  </span>
                </div>
              ))}
              <p className="small muted">
                This balanced entry moves value between accounts. It is not an
                expense or a payment instruction.
              </p>
              {writes &&
                transfer.transferType !== "openingBalance" &&
                !b.journals.some((j) => j.reversesId === transfer.id) && (
                  <button
                    className="button secondary"
                    onClick={() => {
                      setCorrecting(transfer);
                      setTransfer(null);
                    }}
                  >
                    Correct transfer
                  </button>
                )}
              {transfer.transferType === "openingBalance" && (
                <p className="small muted">
                  Correct an opening balance in Settings.
                </p>
              )}
            </div>
          </Modal>
        )}
      </div>
    </SaveErrorContext.Provider>
  );
}
function Overview({
  b,
  period,
  setPeriod,
  currency,
  setCurrency,
  open,
  navigate,
}: {
  b: BusinessData;
  period: string;
  setPeriod: (v: string) => void;
  currency: Currency;
  setCurrency: (v: Currency) => void;
  open: (d: ExpenseDraft) => void;
  navigate: (v: View) => void;
}) {
  const total = totals(b, period),
    categories = categoryTotals(b, period, currency);
  const drafts = b.drafts.filter((d) => d.status === "draft");
  const ready = drafts.filter(
    (d) =>
      !missingDetails(b, d).length &&
      (!duplicates(b, d).length || d.duplicateReason),
  ).length;
  const posted = b.drafts.filter(
    (d) =>
      d.status === "posted" &&
      d.date.startsWith(period) &&
      d.currency === currency,
  );
  const withEvidence = posted.filter((d) => d.documentIds.length).length;
  const max = Math.max(...categories.map((c) => c.amount), 1);
  const recent = [...posted]
    .sort((a, z) => z.date.localeCompare(a.date))
    .slice(0, 4);
  return (
    <>
      <div className="period-row">
        <div className="period-selector">
          <CalendarDays size={16} />
          <input
            aria-label="Reporting period"
            type="month"
            value={period}
            onChange={(e) => {
              if (e.target.value) setPeriod(e.target.value);
            }}
          />
          <select
            aria-label="Reporting currency"
            value={currency}
            onChange={(e) => setCurrency(e.target.value as Currency)}
          >
            {fiatCurrencies.map((c) => (
              <option key={c}>{c}</option>
            ))}
          </select>
        </div>
        <span className="small muted">
          <span className="status-dot" />
          Based on posted records
        </span>
      </div>
      <div className="stats-grid">
        <section className="stat-card principal">
          <div className="stat-label">
            Total spending
            <span>
              <ChartNoAxesCombined size={18} />
            </span>
          </div>
          <div className="stat-value">
            {money(total[currency] ?? 0, currency)}
          </div>
          <p>
            <span className="stat-pill">{currency}</span>Posted expenses this
            period
          </p>
          {Object.entries(total)
            .filter(([c]) => c !== currency)
            .map(([c, n]) => (
              <small key={c}>
                Also {money(n, c as Currency)} · reported separately
              </small>
            ))}
        </section>
        <section className="stat-card">
          <div className="stat-label">
            Awaiting review
            <Inbox size={18} />
          </div>
          <div className="stat-value">
            {drafts.length}
            <span>expenses</span>
          </div>
          <p>
            <span className="status-dot amber" />
            {ready} ready for your review
            <ArrowLink onClick={() => navigate("inbox")}>Open inbox</ArrowLink>
          </p>
        </section>
        <section className="stat-card">
          <div className="stat-label">
            Evidence in place
            <FileText size={18} />
          </div>
          <div className="stat-value">
            {withEvidence}
            <span>of {posted.length} records</span>
          </div>
          <p>
            <span className="status-dot" />
            Posted expenses with a receipt
          </p>
        </section>
      </div>
      <div className="overview-grid">
        <section className="card spending-card">
          <div className="card-heading">
            <div>
              <h2>Where your money goes</h2>
              <p>
                Spending by category <span>· {currency}</span>
              </p>
            </div>
            <span className="chart-symbol">
              <ChartNoAxesCombined size={20} />
            </span>
          </div>
          {categories.length ? (
            <div className="category-chart">
              {categories.map((c, i) => (
                <div className="chart-row" key={c.categoryId}>
                  <div className="chart-label">
                    <span>
                      <i
                        style={{
                          background: [
                            "#1b1f27",
                            "#6b7080",
                            "#a3a8b2",
                            "#d0c8a6",
                            "#d9dcd0",
                          ][i % 5],
                        }}
                      />
                      {c.name}
                    </span>
                    <strong>{money(c.amount, currency)}</strong>
                  </div>
                  <div className="bar-track">
                    <div
                      style={{
                        width: `${(Math.max(0, c.amount) / max) * 100}%`,
                        background: [
                          "#1b1f27",
                          "#6b7080",
                          "#a3a8b2",
                          "#d0c8a6",
                          "#d9dcd0",
                        ][i % 5],
                      }}
                    />
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <Empty title="A fresh start">
              No posted spending in this period and currency.
            </Empty>
          )}
          <div className="card-foot">
            <span>Transfers excluded. Corrections included.</span>
            <ArrowLink onClick={() => navigate("transactions")}>
              View transactions
            </ArrowLink>
          </div>
        </section>
        <section className="attention-card">
          <div className="attention-top">
            <span className="attention-icon">
              <Inbox size={23} strokeWidth={1.6} />
            </span>
            <span className="eyebrow">A LITTLE ATTENTION</span>
          </div>
          <h2>
            {drafts.length ? "Small steps.\nClearer books." : "All caught up."}
          </h2>
          <p>
            {drafts.length
              ? "A few details to check, then you’re one step closer to a clear picture."
              : "Your inbox is clear. New expenses will appear here for review."}
          </p>
          <div className="attention-items">
            <button onClick={() => navigate("inbox")}>
              <span>
                <i className="status-dot" />
                {ready} ready to review
              </span>
              <ArrowRight size={16} />
            </button>
            <button onClick={() => navigate("inbox")}>
              <span>
                <i className="status-dot amber" />
                {drafts.length - ready} need a closer look
              </span>
              <ArrowRight size={16} />
            </button>
          </div>
          <button className="button primary" onClick={() => navigate("inbox")}>
            Go to expense inbox
            <ArrowRight size={16} />
          </button>
          <div className="attention-bottom">
            <ShieldCheck size={14} />
            <span>You review. You stay in control.</span>
          </div>
        </section>
      </div>
      <BalancesPanel b={b} currency={currency} />
      <section className="card recent-card">
        <div className="card-heading">
          <div>
            <h2>Recently posted</h2>
            <p>The latest entries in your books.</p>
          </div>
          <ArrowLink onClick={() => navigate("transactions")}>
            All transactions
          </ArrowLink>
        </div>
        {recent.length ? (
          <ExpenseTable b={b} rows={recent} open={open} compact />
        ) : (
          <Empty title="No entries yet">
            Approve an expense to see it here.
          </Empty>
        )}
      </section>
      <div className="quiet-note">
        <Sparkles size={16} />
        <p>
          A helpful second pair of eyes.{" "}
          <span>
            Demo suggestions help organize the details. Every decision stays
            yours.
          </span>
        </p>
      </div>
    </>
  );
}
/**
 * Each bank, card, loan and equity account's balance in `currency` as of
 * today. A card shows what's owed on it, and a loan what's owed to the
 * owner.
 */
function BalancesPanel({
  b,
  currency,
}: {
  b: BusinessData;
  currency: Currency;
}) {
  const asOf = today();
  const rows = balances(b, asOf, currency);
  return (
    <section className="card recent-card" aria-labelledby="balances-heading">
      <div className="card-heading">
        <div>
          <h2 id="balances-heading">Balances</h2>
          <p>
            As of today, {shortDate(asOf)} <span>· {currency}</span>
          </p>
        </div>
      </div>
      {rows.length ? (
        <div className="balances-list">
          {rows.map((r) => (
            <div className="setting-row" key={r.accountId}>
              <div>
                <strong>{r.name}</strong>
                <small>{accountKindLabels[r.kind]}</small>
              </div>
              <span>
                <small className="small muted">
                  {balanceMeanings[r.kind as keyof typeof balanceMeanings]}
                </small>{" "}
                <strong>{money(r.balance, currency)}</strong>
              </span>
            </div>
          ))}
        </div>
      ) : (
        <Empty title="No accounts in this currency">
          Add a {currency} account in Settings to see its balance.
        </Empty>
      )}
      <div className="card-foot">
        <span>
          Cards and loans show what's owed. Transfers are never spending.
        </span>
      </div>
    </section>
  );
}
function ExpenseTable({
  b,
  rows,
  open,
  compact = false,
  byAssistant,
}: {
  b: BusinessData;
  rows: ExpenseDraft[];
  open: (d: ExpenseDraft) => void;
  compact?: boolean;
  /** Drafts an AI assistant created, by id. */
  byAssistant?: Record<string, string>;
}) {
  return (
    <div className="table-scroll">
      <table className="expenses-table">
        <thead>
          <tr>
            <th>Vendor / description</th>
            <th>Date</th>
            <th>Category</th>
            {!compact && <th>Evidence</th>}
            <th>Status</th>
            <th className="align-right">Amount</th>
            <th>
              <span className="sr-only">Action</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((d) => (
            <tr key={d.id}>
              <td>
                <div className="vendor-cell">
                  <VendorIcon name={d.vendor} />
                  <div>
                    <button className="vendor-name" onClick={() => open(d)}>
                      {d.vendor || "Untitled expense"}
                    </button>
                    <span>{d.description || "Add a description"}</span>
                    {byAssistant?.[d.id] && (
                      <span className="badge blue assistant-mark">
                        Created by an AI assistant
                      </span>
                    )}
                  </div>
                </div>
              </td>
              <td className="nowrap">{shortDate(d.date)}</td>
              <td>
                <span className="category-label">
                  {b.categories.find((c) => c.id === d.categoryId)?.name ??
                    "Uncategorized"}
                </span>
              </td>
              {!compact && (
                <td>
                  <span className="evidence-count">
                    <FileText size={14} />
                    {d.documentIds.length}
                    {d.paymentId && (
                      <span title="Payment matched">
                        <CheckCheck size={14} />
                      </span>
                    )}
                  </span>
                </td>
              )}
              <td>
                <Status b={b} d={d} />
              </td>
              <td className="amount-cell">
                {money(d.amountMinor, d.currency)}
                <small>{d.currency}</small>
              </td>
              <td>
                <button
                  className="row-action"
                  aria-label={`${d.status === "draft" ? "Review" : "View"} ${d.vendor || "expense"}`}
                  onClick={() => open(d)}
                >
                  {d.status === "draft" ? "Review" : <ArrowUpRight size={16} />}
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
function InboxView({
  b,
  open,
  create,
  byAssistant,
}: {
  b: BusinessData;
  open: (d: ExpenseDraft) => void;
  /** Absent for a role that can't create drafts. */
  create?: () => void;
  byAssistant?: Record<string, string>;
}) {
  const [filter, setFilter] = useState("All expenses"),
    [search, setSearch] = useState("");
  const drafts = b.drafts.filter((d) => d.status === "draft");
  const visible = drafts
    .filter((d) =>
      `${d.vendor} ${d.description}`
        .toLowerCase()
        .includes(search.toLowerCase()),
    )
    .filter(
      (d) =>
        filter === "All expenses" ||
        (filter === "Needs details"
          ? missingDetails(b, d).length > 0
          : filter === "Possible duplicates"
            ? duplicates(b, d).length > 0 && !d.duplicateReason
            : !missingDetails(b, d).length &&
              (!duplicates(b, d).length || !!d.duplicateReason)),
    );
  return (
    <>
      <div className="inbox-banner">
        <span className="section-icon">
          <Inbox size={24} />
        </span>
        <div>
          <strong>{drafts.length} expenses, one clear next step.</strong>
          <p>
            Keep receipt evidence and payment evidence together. Approve only
            when the details are right.
          </p>
        </div>
        <span className="badge neutral">Review before posting</span>
      </div>
      <section className="card">
        <div className="list-toolbar">
          <div className="filter-tabs" aria-label="Inbox filter">
            {[
              "All expenses",
              "Ready to review",
              "Needs details",
              "Possible duplicates",
            ].map((f) => (
              <button
                key={f}
                className={filter === f ? "active" : ""}
                aria-pressed={filter === f}
                onClick={() => setFilter(f)}
              >
                {f}
                {f === "All expenses" && <span>{drafts.length}</span>}
              </button>
            ))}
          </div>
          <label className="search-box">
            <Search size={16} />
            <input
              aria-label="Search inbox"
              placeholder="Search expenses…"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          </label>
        </div>
        {visible.length ? (
          <ExpenseTable
            b={b}
            rows={visible}
            open={open}
            byAssistant={byAssistant}
          />
        ) : (
          <Empty
            title={
              drafts.length
                ? "No matching expenses"
                : "A clear inbox. A good feeling."
            }
          >
            {drafts.length ? (
              "Try another search or filter."
            ) : (
              <>
                {create && (
                  <>
                    <span>Create an expense to start a new review.</span>
                    <button className="button primary" onClick={create}>
                      <Plus size={16} />
                      New expense
                    </button>
                  </>
                )}
              </>
            )}
          </Empty>
        )}
        <div className="card-foot">
          <span>
            {visible.length} of {drafts.length} expenses
          </span>
          <span>Drafts do not affect reports.</span>
        </div>
      </section>
    </>
  );
}
function Transactions({
  b,
  open,
  openTransfer,
}: {
  b: BusinessData;
  open: (d: ExpenseDraft) => void;
  openTransfer: (j: JournalEntry) => void;
}) {
  const [search, setSearch] = useState(""),
    [filter, setFilter] = useState("All records");
  const rows = b.drafts
    .filter(
      (d) =>
        d.status !== "draft" &&
        `${d.vendor} ${d.description} ${b.categories.find((c) => c.id === d.categoryId)?.name ?? ""}`
          .toLowerCase()
          .includes(search.toLowerCase()),
    )
    .filter(
      (d) =>
        filter === "All records" ||
        (filter === "Corrections" &&
          (d.status === "corrected" || d.correctionOf)),
    )
    .sort((a, z) => z.date.localeCompare(a.date));
  const transfers = b.journals
    .filter(
      (j) =>
        j.kind === "transfer" &&
        `${j.memo} ${j.purpose ?? ""} ${j.transferType ? transferTypeLabels[j.transferType] : ""}`
          .toLowerCase()
          .includes(search.toLowerCase()) &&
        filter !== "Corrections",
    )
    .sort((a, z) => z.date.localeCompare(a.date));
  return (
    <section className="card">
      <div className="list-toolbar">
        <div className="filter-tabs">
          {["All records", "Corrections", "Transfers"].map((f) => (
            <button
              key={f}
              aria-pressed={f === filter}
              className={f === filter ? "active" : ""}
              onClick={() => setFilter(f)}
            >
              {f}
            </button>
          ))}
        </div>
        <label className="search-box">
          <Search size={16} />
          <input
            placeholder="Search transactions…"
            aria-label="Search transactions"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </label>
      </div>
      {filter !== "Transfers" && rows.length > 0 && (
        <ExpenseTable b={b} rows={rows} open={open} />
      )}{" "}
      {transfers.map((t) => (
        <button
          className="transfer-row"
          key={t.id}
          onClick={() => openTransfer(t)}
        >
          <span className="vendor-icon sage">
            <ArrowLeftRight size={18} />
          </span>
          <span>
            <strong>{t.memo}</strong>
            <small>
              {shortDate(t.date)} ·{" "}
              {t.transferType
                ? transferTypeLabels[t.transferType]
                : "Between business accounts"}
              {t.purpose &&
                t.purpose !==
                  (t.transferType && transferTypeLabels[t.transferType]) &&
                ` · ${t.purpose}`}
            </small>
          </span>
          <span className="badge blue">
            {b.journals.some((j) => j.reversesId === t.id)
              ? "Corrected"
              : "Transfer"}
          </span>
          <strong>{money(t.lines[0].debit, t.currency)}</strong>
          <ArrowUpRight size={16} />
        </button>
      ))}
      {(!rows.length || filter === "Transfers") && !transfers.length && (
        <Empty title="No matching records">
          Try another search or post an expense from the inbox.
        </Empty>
      )}
      <div className="card-foot">
        <span>Original entries remain visible after a correction.</span>
        <span>
          <ShieldCheck size={14} />
          Balanced, traceable records
        </span>
      </div>
    </section>
  );
}
