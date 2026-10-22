import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input, fieldClass } from "@/components/ui/input";
import {
  qoderAuthApi,
  type QoderAuthProfileInput,
  type QoderAuthProfiles,
} from "@/lib/api/qoderAuth";

export function QoderAuthSection() {
  const { t } = useTranslation();
  const prefix = useId();
  const [store, setStore] = useState<QoderAuthProfiles>({
    profiles: [],
    selectedId: null,
  });
  const [pending, setPending] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | undefined>();
  const [label, setLabel] = useState("");
  const [cliPath, setCliPath] = useState("");
  const [nodePath, setNodePath] = useState("");
  const [source, setSource] = useState<"pat_env" | "cli">("cli");
  const [reference, setReference] = useState("");
  const text = (key: string, fallback: string) =>
    t(`settings.qoderAuth.${key}`, fallback);

  useEffect(() => {
    let current = true;
    qoderAuthApi
      .detect()
      .then((runtime) => {
        if (!current) return;
        setCliPath(runtime.cliPath ?? "");
        setReference(runtime.configDir ?? "");
      })
      .catch((reason) => {
        if (current) setError(String(reason));
      });
    qoderAuthApi
      .list()
      .then((value) => {
        if (current) setStore(value);
      })
      .catch((reason) => {
        if (current) setError(String(reason));
      })
      .finally(() => {
        if (current) setPending(false);
      });
    return () => {
      current = false;
    };
  }, []);

  const run = async (operation: () => Promise<QoderAuthProfiles | void>) => {
    setPending(true);
    setError(null);
    setNotice(null);
    try {
      const updated = await operation();
      if (updated) setStore(updated);
      return true;
    } catch (reason) {
      setError(String(reason));
      return false;
    } finally {
      setPending(false);
    }
  };

  const reset = () => {
    setEditing(undefined);
    setLabel("");
    setCliPath("");
    setNodePath("");
    setReference("");
  };
  const save = async () => {
    const input: QoderAuthProfileInput = {
      id: editing,
      label,
      cliPath,
      nodePath: nodePath.trim() || undefined,
      source:
        source === "pat_env"
          ? { type: source, envVar: reference }
          : { type: source, configDir: reference },
    };
    if (await run(() => qoderAuthApi.save(input))) reset();
  };

  return (
    <section
      aria-labelledby={`${prefix}-title`}
      className="rounded-card border border-border bg-surface p-4 space-y-3"
    >
      <h3 id={`${prefix}-title`} className="font-semibold">
        Qoder CN
      </h3>
      <p className="text-caption text-fg-2">
        {text(
          "scope",
          "Manage explicit authorization profiles. Selection does not switch the desktop account. Official CLI login/logout remains separate; unlinking here does not revoke credentials.",
        )}
      </p>
      {error && (
        <p role="alert" className="text-danger">
          {error}
        </p>
      )}
      {notice && <p role="status">{notice}</p>}
      <ul className="space-y-2">
        {store.profiles.map((profile) => (
          <li
            key={profile.id}
            className="rounded-control border border-border p-3 space-y-2"
          >
            <p className="font-medium">
              {profile.label}
              {store.selectedId === profile.id
                ? ` · ${text("selected", "Selected profile")}`
                : ""}
            </p>
            <p className="text-caption text-fg-2">
              {profile.account
                ? `${profile.account.name || profile.account.userId} · ${profile.account.userId}`
                : text("unverified", "Account not verified")}
              {profile.verifiedAt &&
                ` · ${text("lastVerified", "Last verified")}: ${new Date(profile.verifiedAt).toLocaleString()}`}
            </p>
            {profile.lastError && (
              <p role="status" className="text-danger">
                {profile.lastError}
              </p>
            )}
            <div className="flex flex-wrap gap-2">
              {profile.source.type === "cli" && (
                <Button
                  variant="neutral"
                  disabled={pending}
                  onClick={() => {
                    void run(() => qoderAuthApi.openCli(profile.id)).then(
                      (opened) => {
                        if (opened)
                          setNotice(
                            text(
                              "cliRequested",
                              "Account terminal launch requested. Complete /login, /status or /logout there, then verify this profile. This does not confirm login or switch the desktop account.",
                            ),
                          );
                      },
                    );
                  }}
                >
                  {text("openCli", "Open official account CLI")}
                </Button>
              )}
              <Button
                variant="neutral"
                disabled={pending}
                onClick={() => void run(() => qoderAuthApi.verify(profile.id))}
              >
                {text("verify", "Verify account (no inference)")}
              </Button>
              <Button
                variant="neutral"
                disabled={
                  pending ||
                  !profile.account ||
                  !!profile.lastError ||
                  store.selectedId === profile.id
                }
                onClick={() => void run(() => qoderAuthApi.select(profile.id))}
              >
                {text("select", "Select profile")}
              </Button>
              <Button
                variant="quiet"
                disabled={pending}
                onClick={() => {
                  setEditing(profile.id);
                  setLabel(profile.label);
                  setCliPath(profile.cliPath);
                  setNodePath(profile.nodePath || "");
                  setSource(profile.source.type);
                  setReference(
                    profile.source.type === "pat_env"
                      ? profile.source.envVar
                      : profile.source.configDir,
                  );
                }}
              >
                {text("edit", "Edit / rebind")}
              </Button>
              <Button
                variant="quiet"
                disabled={pending}
                onClick={() => void run(() => qoderAuthApi.remove(profile.id))}
              >
                {text("unlink", "Unlink profile")}
              </Button>
            </div>
          </li>
        ))}
      </ul>
      {store.selectedId && (
        <Button
          variant="neutral"
          disabled={pending}
          onClick={() => void run(() => qoderAuthApi.select(null))}
        >
          {text("deselect", "Clear profile selection")}
        </Button>
      )}
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
        className="grid gap-3"
      >
        <fieldset disabled={pending} className="grid gap-3">
          <legend className="text-body font-medium mb-2">
            {editing
              ? text("edit", "Edit / rebind")
              : text("add", "Add authorization profile")}
          </legend>
          <label htmlFor={`${prefix}-label`}>
            {text("label", "Profile name")}
          </label>
          <Input
            id={`${prefix}-label`}
            value={label}
            onChange={(event) => setLabel(event.target.value)}
            required
            maxLength={256}
          />
          <label htmlFor={`${prefix}-cli`}>
            {text(
              "cliPath",
              "Qoder CN CLI (automatically detected; override only if needed)",
            )}
          </label>
          <Input
            id={`${prefix}-cli`}
            value={cliPath}
            onChange={(event) => setCliPath(event.target.value)}
            required
          />
          <label htmlFor={`${prefix}-node`}>
            {text(
              "nodePath",
              "Node executable (optional; otherwise resolved from PATH)",
            )}
          </label>
          <Input
            id={`${prefix}-node`}
            value={nodePath}
            onChange={(event) => setNodePath(event.target.value)}
          />
          <label htmlFor={`${prefix}-source`}>
            {text("source", "Authorization source")}
          </label>
          <select
            id={`${prefix}-source`}
            value={source}
            onChange={(event) => {
              setSource(event.target.value as "pat_env" | "cli");
              setReference("");
            }}
            className={`${fieldClass} h-8`}
          >
            <option value="pat_env">
              {text("pat", "Official PAT environment reference")}
            </option>
            <option value="cli">
              {text("native", "Explicit native CLI login directory")}
            </option>
          </select>
          <label htmlFor={`${prefix}-reference`}>
            {source === "pat_env"
              ? text("env", "Environment variable name (not the token)")
              : text(
                  "configDir",
                  "Native CLI configuration directory (absolute path)",
                )}
          </label>
          <Input
            id={`${prefix}-reference`}
            value={reference}
            onChange={(event) => setReference(event.target.value)}
            required
          />
          <div className="flex gap-2">
            <Button type="submit" variant="neutral">
              {text("save", "Save profile reference")}
            </Button>
            {editing && (
              <Button type="button" variant="quiet" onClick={reset}>
                {text("cancel", "Cancel")}
              </Button>
            )}
          </div>
        </fieldset>
      </form>
    </section>
  );
}
