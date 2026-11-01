import { lazy, Suspense, useState } from "react";
import App from "./App";
import { SignIn } from "./components/SignIn";
import { serviceConfigured } from "./remote/config";
import { demoOnly } from "./demo";
import type { RemoteLedgerClient } from "./remote";

// The ledger service's screens load only after sign-in, with the protocol.
const RemoteWorkspace = lazy(() => import("./RemoteWorkspace"));

/** Above a demo-only build: what it is, and the way back to the site. */
function DemoBanner() {
  return (
    <div className="demo-banner" role="note">
      <span>Demo with fictional data. Everything stays in your browser.</span>
      <a href="/">About Zap Books</a>
    </div>
  );
}

/**
 * Without a configured ledger service, the app is the local demo. With one,
 * it starts at sign-in, and the local demo stays a click away.
 */
export default function Root() {
  const [mode, setMode] = useState<"start" | "local">("start");
  const [remote, setRemote] = useState<RemoteLedgerClient>();
  if (demoOnly)
    return (
      <>
        <DemoBanner />
        <App />
      </>
    );
  if (!serviceConfigured || mode === "local") return <App />;
  if (!remote)
    return <SignIn onSignedIn={setRemote} onLocal={() => setMode("local")} />;
  return (
    <Suspense fallback={<main className="loading">Opening your books…</main>}>
      <RemoteWorkspace
        client={remote}
        onSignOut={() => {
          remote.api.signOut();
          setRemote(undefined);
        }}
      />
    </Suspense>
  );
}
