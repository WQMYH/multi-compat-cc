import { invoke } from "@tauri-apps/api/core";

export interface PlanAccount {
  id: string;
  label: string;
  phase:
    | "signed_out"
    | "connected"
    | "reauth_required"
    | "exchange_unknown"
    | "refreshing"
    | "refresh_unknown";
  identity: {
    issuer: string;
    clientId: string;
    subject: string;
    email: string | null;
    inferenceAuthorized: boolean;
    expiresAt: number;
  } | null;
  lastError: string | null;
}

export interface PlanLogin {
  attemptId: string;
  accountId: string;
}

export interface PlanAccounts {
  accounts: PlanAccount[];
  activeId: string | null;
  pending: PlanLogin | null;
}

export const chatgptPlanApi = {
  status: () => invoke<PlanAccounts>("chatgpt_plan_status"),
  startLogin: (label: string, accountId?: string) =>
    invoke<PlanLogin>("chatgpt_plan_start_login", { label, accountId }),
  cancelLogin: (attemptId: string) =>
    invoke<PlanAccounts>("chatgpt_plan_cancel_login", { attemptId }),
  select: (accountId: string | null) =>
    invoke<PlanAccounts>("chatgpt_plan_select", { accountId }),
  signOut: (accountId: string) =>
    invoke<PlanAccounts>("chatgpt_plan_sign_out", { accountId }),
};
