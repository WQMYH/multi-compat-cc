import { invoke } from "@tauri-apps/api/core";

export type QoderAuthSource =
  | { type: "pat_env"; envVar: string }
  | { type: "cli"; configDir: string };

export interface QoderAuthProfileInput {
  id?: string;
  label: string;
  source: QoderAuthSource;
  cliPath: string;
  nodePath?: string;
}

export interface QoderAuthProfile extends QoderAuthProfileInput {
  id: string;
  revision: number;
  account: {
    userId: string;
    name?: string | null;
    email?: string | null;
    organizationName?: string | null;
    subscriptionType?: string | null;
  } | null;
  verifiedAt: string | null;
  lastError: string | null;
}

export interface QoderAuthProfiles {
  profiles: QoderAuthProfile[];
  selectedId: string | null;
}

export const qoderAuthApi = {
  detect: () =>
    invoke<{ cliPath: string | null; configDir: string | null }>(
      "qoder_auth_detect_runtime",
    ),
  list: () => invoke<QoderAuthProfiles>("qoder_auth_list_profiles"),
  save: (input: QoderAuthProfileInput) =>
    invoke<QoderAuthProfiles>("qoder_auth_save_profile", { input }),
  select: (id: string | null) =>
    invoke<QoderAuthProfiles>("qoder_auth_select_profile", { id }),
  verify: (id: string) =>
    invoke<QoderAuthProfiles>("qoder_auth_verify_profile", { id }),
  remove: (id: string) =>
    invoke<QoderAuthProfiles>("qoder_auth_remove_profile", { id }),
  openCli: (id?: string) => invoke<void>("qoder_auth_open_cli", { id }),
};
