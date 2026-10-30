import { useState } from "react";
import App from "./App";
import { CreateBusiness } from "./components/CreateBusiness";
import { npub, type RemoteLedgerClient } from "./remote";

/** A signed-in member's businesses on the ledger service. */
export default function RemoteWorkspace({
  client,
  onSignOut,
}: {
  client: RemoteLedgerClient;
  onSignOut: () => void;
}) {
  const [creating, setCreating] = useState(
    () => !client.workspace().businesses.length,
  );
  if (creating)
    return (
      <CreateBusiness
        client={client}
        onCreated={(id) => {
          try {
            localStorage.setItem("business-finance-active", id);
          } catch {
            /* Preference persistence is optional. */
          }
          setCreating(false);
        }}
        onCancel={
          client.workspace().businesses.length
            ? () => setCreating(false)
            : undefined
        }
      />
    );
  return (
    <App
      key={client.signer.pubkey}
      remote={client}
      remoteNpub={npub(client.signer.pubkey)}
      onSignOut={onSignOut}
      onNewBusiness={() => setCreating(true)}
    />
  );
}
