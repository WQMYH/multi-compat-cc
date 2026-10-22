import { invoke } from "@tauri-apps/api/core";

export type QoderApiFormat = "openai_chat" | "openai_responses";
export interface QoderGatewayProvider {
  id: string;
  name: string;
  baseUrl: string;
  apiFormat: QoderApiFormat;
  models: string[];
  planAccountId?: string;
  codexAccountId?: string;
}
export interface QoderGatewayStatus {
  selectedId: string | null;
  keyConfigured: boolean;
  providers: QoderGatewayProvider[];
}
export interface QoderGatewayInput extends Omit<QoderGatewayProvider, "id"> {
  id?: string;
  apiKey?: string;
}
export interface QoderPlanModel {
  id: string;
  displayName: string;
}
export const qoderGatewayApi = {
  status: () => invoke<QoderGatewayStatus>("qoder_gateway_status"),
  save: (input: QoderGatewayInput) =>
    invoke<QoderGatewayStatus>("qoder_gateway_save", { input }),
  select: (id: string) =>
    invoke<QoderGatewayStatus>("qoder_gateway_select", { id }),
  remove: (id: string) =>
    invoke<QoderGatewayStatus>("qoder_gateway_remove", { id }),
  prepareKey: () => invoke<string>("qoder_gateway_prepare_key"),
  planModels: (accountId: string) =>
    invoke<QoderPlanModel[]>("qoder_gateway_plan_models", { accountId }),
};
