import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { qoderAuthApi } from "@/lib/api/qoderAuth";
import { QoderGatewaySection } from "@/components/settings/QoderGatewaySection";
import { QoderAuthSection } from "@/components/settings/QoderAuthSection";

export function QoderApplication({
  onManageAccounts,
}: {
  onManageAccounts: () => void;
}) {
  const [cli, setCli] = useState("正在自动检测 Qoder CN CLI…");
  useEffect(() => {
    let active = true;
    qoderAuthApi
      .detect()
      .then((runtime) => {
        if (active)
          setCli(
            runtime.cliPath ??
              "未检测到 qodercn；可在「应用」中安装，或在授权档案中指定例外路径。",
          );
      })
      .catch((error) => {
        if (active) setCli(String(error));
      });
    return () => {
      active = false;
    };
  }, []);
  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-card border border-border bg-surface p-4">
        <div>
          <h2 className="font-semibold">模型供应商与账号</h2>
          <p className="text-caption text-fg-2">
            复用 CC Switch 的 Codex 账号；Qoder
            官方账号与第三方模型来源分别管理。
          </p>
          <p className="text-caption text-fg-2 break-all" role="status">
            CLI 自动检测：{cli}
          </p>
        </div>
        <Button variant="neutral" onClick={onManageAccounts}>
          管理 Codex 账号
        </Button>
      </div>
      <QoderGatewaySection />
      <details className="rounded-card border border-border bg-surface p-4">
        <summary className="cursor-pointer font-medium">
          Qoder 官方账号与授权档案
        </summary>
        <div className="mt-4">
          <QoderAuthSection />
        </div>
      </details>
    </div>
  );
}
