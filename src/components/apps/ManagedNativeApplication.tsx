import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ProviderList } from "@/components/providers/ProviderList";
import { AddProviderDialog } from "@/components/providers/AddProviderDialog";
import { EditProviderDialog } from "@/components/providers/EditProviderDialog";
import { useProvidersQuery } from "@/lib/query";
import { useProviderActions } from "@/hooks/useProviderActions";
import { toast } from "@/lib/toast";
import { settingsApi } from "@/lib/api";
import type { Provider } from "@/types";

type SourceApp = "codex" | "claude";
interface NativeStatus {
  cliPath: string | null;
  dataDir: string | null;
  detectionError: string | null;
  provider: { sourceApp: SourceApp; providerId: string } | null;
  readonlyPreview: boolean;
}

export function ManagedNativeApplication({
  target,
  source,
  adding,
  onAddingChange,
  searchOpen,
  onSearchOpenChange,
  onManageSource,
}: {
  target: "qoder" | "dsh";
  source: SourceApp;
  adding: boolean;
  onAddingChange: (open: boolean) => void;
  searchOpen?: boolean;
  onSearchOpenChange?: (open: boolean) => void;
  onManageSource: (source: SourceApp) => void;
}) {
  const queryClient = useQueryClient();
  const [editing, setEditing] = useState<Provider | null>(null);
  const { data, isLoading, error } = useProvidersQuery(source);
  const { addProvider, updateProvider } = useProviderActions(source, false);
  const status = useQuery({
    queryKey: ["nativeApplication", target],
    queryFn: () =>
      invoke<NativeStatus>("native_application_status", { target }),
    retry: false,
  });
  const readonly = status.data?.readonlyPreview !== false;
  const select = async (provider: Provider) => {
    if (readonly) {
      toast.info("初验只读：不会切换账号、写入应用配置或调用模型。");
      return;
    }
    try {
      await invoke("native_application_select_provider", {
        target,
        provider: { sourceApp: source, providerId: provider.id },
      });
      await queryClient.invalidateQueries({
        queryKey: ["nativeApplication", target],
      });
      toast.success("已关联 CC 托管供应商；尚未写入原生应用配置。");
    } catch (error) {
      toast.error(String(error));
    }
  };
  const editSource = () => onManageSource(source);
  return (
    <>
      {error && <p role="alert">供应商读取失败：{String(error)}</p>}
      <ProviderList
        key={source}
        appId={source}
        providers={data?.providers ?? {}}
        isLoading={isLoading}
        currentProviderId={
          status.data?.provider?.sourceApp === source
            ? status.data.provider.providerId
            : ""
        }
        onSwitch={(provider) => void select(provider)}
        onEdit={setEditing}
        onDelete={editSource}
        onDuplicate={editSource}
        onOpenWebsite={(url) => void settingsApi.openExternal(url)}
        onCreate={() => onAddingChange(true)}
        searchOpen={searchOpen}
        onSearchOpenChange={onSearchOpenChange}
        switchMode={{
          active: "direct",
          view: "direct",
          directId:
            status.data?.provider?.sourceApp === source
              ? status.data.provider.providerId
              : null,
          routeId: null,
          failoverOn: false,
          queue: [],
          stackMembers: new Map(),
          serviceRunning: false,
          routingReason: () =>
            "沿用 CC 已托管的账号与供应商路由，无需重复认证。",
          actions: {
            switchDirect: (provider) => void select(provider),
            needsRouteDialog: (provider) => void select(provider),
            exitAndUse: (provider) => void select(provider),
            routeTo: (provider) => void select(provider),
            queueAdd: editSource,
            queueRemove: editSource,
            queueMove: editSource,
            stackAdd: editSource,
            stackRemove: editSource,
            stackSetDefault: editSource,
          },
        }}
      />
      <AddProviderDialog
        open={adding}
        onOpenChange={onAddingChange}
        appId={source}
        onSubmit={async (provider) => {
          if (readonly) throw new Error("只读初验不保存配置");
          await addProvider(provider);
        }}
      />
      <EditProviderDialog
        open={Boolean(editing)}
        provider={editing}
        appId={source}
        onOpenChange={(open) => {
          if (!open) setEditing(null);
        }}
        onSubmit={async ({ provider, originalId, editorSave }) => {
          if (readonly) throw new Error("只读初验不保存配置");
          await updateProvider(provider, originalId, editorSave);
        }}
      />
    </>
  );
}
