import { isMac, isWindows } from "@/lib/platform";
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

/** 能不能切 agy 的全局登录：和后端 `agy_account::keychain::supported` 一致（macOS 钥匙串、Windows 凭据管理器）。 */
export function canSwitchAgyAccount(): boolean {
  return isMac() || isWindows();
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
