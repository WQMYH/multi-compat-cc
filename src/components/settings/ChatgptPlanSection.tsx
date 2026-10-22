import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { chatgptPlanApi, type PlanAccounts } from "@/lib/api/chatgptPlan";

export function ChatgptPlanSection() {
  const { t } = useTranslation();
  const nameId = useId();
  const [store, setStore] = useState<PlanAccounts>({
    accounts: [],
    activeId: null,
    pending: null,
  });
  const [label, setLabel] = useState("");
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const text = (key: string, fallback: string) =>
    t(`settings.chatgptPlan.${key}`, fallback);

  useEffect(() => {
    let current = true;
    chatgptPlanApi
      .status()
      .then((value) => {
        if (current) setStore(value);
      })
      .catch((reason) => {
        if (current) setError(String(reason));
      })
      .finally(() => {
        if (current) setBusy(false);
      });
    return () => {
      current = false;
    };
  }, []);

  useEffect(() => {
    if (!store.pending || busy) return;
    let current = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const value = await chatgptPlanApi.status();
        if (current) {
          setStore(value);
          setError(null);
        }
      } catch (reason) {
        if (current) setError(String(reason));
      }
      if (current) timer = setTimeout(() => void poll(), 1000);
    };
    timer = setTimeout(() => void poll(), 1000);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [store.pending?.attemptId, busy]);

  const run = async (action: () => Promise<PlanAccounts>) => {
    setBusy(true);
    setError(null);
    try {
      setStore(await action());
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };
  const start = (name: string, accountId?: string) =>
    run(async () => {
      await chatgptPlanApi.startLogin(name, accountId);
      return chatgptPlanApi.status();
    });
  const locked = busy || !!store.pending;

  return (
    <section className="grid gap-3 rounded-lg border border-stroke-2 p-4">
      <h3 className="text-base font-semibold">
        {text("title", "ChatGPT plan authorization for Qoder")}
      </h3>
      <p className="text-sm text-fg-2">
        {text(
          "description",
          "Explicit plan authorization, separate from native Codex accounts. No inference is run during sign-in.",
        )}
      </p>
      {error && <p role="alert">{error}</p>}
      {store.pending && (
        <div role="status" className="flex items-center gap-2">
          <span>{text("pending", "Browser sign-in in progress")}</span>
          <Button
            variant="neutral"
            disabled={busy}
            onClick={() =>
              void run(() =>
                chatgptPlanApi.cancelLogin(store.pending!.attemptId),
              )
            }
          >
            {text("cancel", "Cancel sign-in")}
          </Button>
        </div>
      )}
      <ul className="grid gap-3">
        {store.accounts.map((account) => (
          <li
            key={account.id}
            className="grid gap-2 rounded border border-stroke-2 p-3"
          >
            <p className="font-medium">
              {account.label} · {account.id.slice(0, 8)}
              {store.activeId === account.id
                ? ` · ${text("selected", "Selected")}`
                : ""}
            </p>
            {account.identity?.email && (
              <p className="text-sm">{account.identity.email}</p>
            )}
            <p className="text-sm">
              {account.phase === "connected"
                ? account.identity?.inferenceAuthorized
                  ? text("authorized", "Signed in; plan use authorized")
                  : text("notAuthorized", "Signed in; plan use not authorized")
                : account.phase}
            </p>
            {account.lastError && (
              <p role="status" className="text-sm">
                {account.lastError}
              </p>
            )}
            <div className="flex flex-wrap gap-2">
              <Button
                variant="neutral"
                disabled={
                  locked ||
                  account.phase !== "connected" ||
                  !account.identity ||
                  store.activeId === account.id
                }
                onClick={() =>
                  void run(() => chatgptPlanApi.select(account.id))
                }
              >
                {text("select", "Select account")}
              </Button>
              <Button
                variant="neutral"
                disabled={locked}
                onClick={() => void start(account.label, account.id)}
              >
                {text("reconnect", "Authorize again")}
              </Button>
              <Button
                variant="quiet"
                disabled={busy || account.phase === "signed_out"}
                onClick={() =>
                  void run(() => chatgptPlanApi.signOut(account.id))
                }
              >
                {text("signOut", "Sign out; keep registration")}
              </Button>
            </div>
          </li>
        ))}
      </ul>
      <form
        className="flex flex-wrap items-end gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          void start(label);
        }}
      >
        <label htmlFor={nameId}>
          {text("name", "Registration name")}
          <Input
            id={nameId}
            value={label}
            disabled={locked}
            maxLength={256}
            onChange={(event) => setLabel(event.target.value)}
          />
        </label>
        <Button
          variant="neutral"
          type="submit"
          disabled={locked || !label.trim()}
        >
          {text("continue", "Continue with ChatGPT")}
        </Button>
      </form>
      <p className="text-xs text-fg-2">
        {text(
          "scope",
          "Only eligible Responses requests can use this grant. Selecting it does not alter Codex settings or Qoder's official login.",
        )}
      </p>
    </section>
  );
}
