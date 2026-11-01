// The beta form, on Formspree. Without this script the form is an ordinary
// post; with it, the form checks the address first, sends the same fields
// with fetch, and offers a Nostr public key instead of an email. The field
// is named for what it holds, `email` or `npub`, and `contact_type` says
// which.

const MODES = {
  email: {
    label: "Email address",
    type: "email",
    autocomplete: "email",
    placeholder: "you@business.com",
    toggle: "Use a Nostr public key instead",
  },
  npub: {
    label: "Nostr public key (npub)",
    type: "text",
    autocomplete: "off",
    placeholder: "npub1…",
    toggle: "Use an email address instead",
  },
};

/** The error to show for `value`, or "" when it can be sent. */
function validate(mode, value) {
  const v = value.trim();
  if (mode === "email") {
    if (!v) return "Enter your email address.";
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(v))
      return "Enter a valid email address, like you@business.com.";
    return "";
  }
  if (!v) return "Enter your Nostr public key.";
  // bech32: "npub1", then 58 characters of its alphabet (no 1, b, i or o).
  if (!/^npub1[02-9ac-hj-np-z]{58}$/.test(v))
    return "Enter a Nostr public key that starts with npub1 and is 63 characters long.";
  return "";
}

function start() {
  const form = document.getElementById("beta-form");
  if (!form) return;
  const input = document.getElementById("beta-contact");
  const label = document.getElementById("beta-label");
  const kind = form.elements.namedItem("contact_type");
  const submit = document.getElementById("beta-submit");
  const toggle = document.getElementById("beta-mode");
  const error = document.getElementById("beta-error");
  const done = document.getElementById("beta-done");
  const contactEmail = form.dataset.contactEmail;
  let mode = "email";

  form.noValidate = true;
  toggle.hidden = false;

  /** Shows one paragraph per message; each is a string or a list of nodes. */
  function showError(...messages) {
    error.replaceChildren(
      ...messages.map((parts) => {
        const p = document.createElement("p");
        p.append(...[parts].flat());
        return p;
      }),
    );
  }

  function showUnsent() {
    const link = document.createElement("a");
    link.href = `mailto:${contactEmail}`;
    link.textContent = contactEmail;
    showError([
      "Couldn't send your request. Try again, or email us at ",
      link,
      ".",
    ]);
  }

  /**
   * Sends the form as Formspree's AJAX API expects. Returns "sent", the
   * messages of an `errors` answer, or null when it couldn't be sent.
   */
  async function send() {
    try {
      const response = await fetch(form.action, {
        method: "POST",
        headers: { Accept: "application/json" },
        body: new FormData(form),
      });
      const body = await response.json().catch(() => null);
      if (response.ok && body?.ok) return "sent";
      const messages = Array.isArray(body?.errors)
        ? body.errors
            .map((e) => (typeof e?.message === "string" ? e.message : ""))
            .filter(Boolean)
        : [];
      return messages.length ? messages : null;
    } catch {
      return null;
    }
  }

  function clearError() {
    error.replaceChildren();
    input.removeAttribute("aria-invalid");
  }

  function setMode(next) {
    mode = next;
    const m = MODES[mode];
    label.textContent = m.label;
    input.type = m.type;
    input.autocomplete = m.autocomplete;
    input.placeholder = m.placeholder;
    input.value = "";
    input.name = mode;
    kind.value = mode;
    toggle.textContent = m.toggle;
    clearError();
  }

  input.addEventListener("input", clearError);
  toggle.addEventListener("click", () =>
    setMode(mode === "email" ? "npub" : "email"),
  );

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const problem = validate(mode, input.value);
    if (problem) {
      input.setAttribute("aria-invalid", "true");
      showError(problem);
      input.focus();
      return;
    }
    const value = input.value.trim();
    input.value = value;
    clearError();
    submit.disabled = true;
    submit.textContent = "Sending…";
    const result = await send();
    submit.disabled = false;
    submit.textContent = "Request an invite";
    if (result === null) return showUnsent();
    if (result !== "sent") return showError(...result);
    document.getElementById("beta-done-text").textContent =
      `We'll contact you at ${value} when invites go out.`;
    form.hidden = true;
    done.hidden = false;
    document.getElementById("beta-done-title").focus();
  });
}

start();
