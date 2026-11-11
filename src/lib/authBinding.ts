import type { ProviderMeta } from "@/types";
import type { AppId } from "@/lib/api/types";

export function isDshCodexOAuthProvider(
  appId: AppId,
  meta: ProviderMeta | undefined,
): boolean {
  return (
    appId === "dsh" &&
    (meta?.providerType === "codex_oauth" ||
      meta?.authBinding?.source === "managed_account" ||
      meta?.authBinding?.authProvider === "codex_oauth")
  );
}

export function resolveManagedAccountId(
  meta: ProviderMeta | undefined,
  authProvider: string,
): string | null {
  const binding = meta?.authBinding;

  if (
    binding?.source === "managed_account" &&
    binding.authProvider === authProvider
  ) {
    return binding.accountId ?? null;
  }

  if (authProvider === "github_copilot") {
    return meta?.githubAccountId ?? null;
  }

  return null;
}
