import { useCallback, useState } from "react";
import App from "./App";
import { ReceiptFiles } from "./components/Review";
import type { SourceDocument } from "./domain/types";
import { CreateBusiness } from "./components/CreateBusiness";
import { RecoveryMode } from "./components/RecoveryMode";
import { npub, peopleTools, type RemoteLedgerClient } from "./remote";

type Screen = "books" | "create" | "recovery";

/** A signed-in member's businesses on the ledger service. */
export default function RemoteWorkspace({
  client,
  onSignOut,
}: {
  client: RemoteLedgerClient;
  onSignOut: () => void;
}) {
  const receiptFile = useCallback(
    (d: SourceDocument) => client.receiptFile(d.businessId, d),
    [client],
  );
  const hasBooks = () => client.workspace().businesses.length > 0;
  const [screen, setScreen] = useState<Screen>(() =>
    hasBooks() ? "books" : client.guarded.length ? "recovery" : "create",
  );
  if (screen === "recovery")
    return (
      <RecoveryMode
        client={client}
        tools={peopleTools(client, "")}
        onSignOut={onSignOut}
        onBack={hasBooks() ? () => setScreen("books") : undefined}
      />
    );
  const stopped = [...client.sync.stopped.values()];
  if (screen === "create" && !hasBooks() && stopped.length)
    return (
      <main className="loading sign-in">
        <section className="card sign-in-card">
          <h2>Your books couldn't be opened</h2>
          {stopped.map((m) => (
            <p key={m} role="alert" className="alert error">
              {m}
            </p>
          ))}
          <button className="button secondary" onClick={onSignOut}>
            Sign out
          </button>
        </section>
      </main>
    );
  if (screen === "create")
    return (
      <CreateBusiness
        client={client}
        onCreated={(id) => {
          try {
            localStorage.setItem("business-finance-active", id);
          } catch {
            /* Preference persistence is optional. */
          }
          setScreen("books");
        }}
        onCancel={hasBooks() ? () => setScreen("books") : undefined}
      />
    );
  return (
    <ReceiptFiles.Provider value={receiptFile}>
      <App
        key={client.signer.pubkey}
        remote={client}
        remoteNpub={npub(client.signer.pubkey)}
        peopleTools={(business) => peopleTools(client, business)}
        onSignOut={onSignOut}
        onNewBusiness={() => setScreen("create")}
        onRecoveryMode={
          client.guarded.length ? () => setScreen("recovery") : undefined
        }
      />
    </ReceiptFiles.Provider>
  );
}
