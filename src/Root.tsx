import { lazy, Suspense, useState } from "react";
import App from "./App";
import { SignIn } from "./components/SignIn";
import { serviceConfigured } from "./remote/config";
import type { RemoteLedgerClient } from "./remote";

// The ledger service's screens load only after sign-in, with the protocol.
const RemoteWorkspace = lazy(() => import("./RemoteWorkspace"));

/**
 * Without a configured ledger service, the app is the local demo. With one,
 * it starts at sign-in, and the local demo stays a click away.
 */
export default function Root() {
  const [mode, setMode] = useState<"start" | "local">("start");
  const [remote, setRemote] = useState<RemoteLedgerClient>();
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
