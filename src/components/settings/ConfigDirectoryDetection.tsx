import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { settingsApi, type AppId } from "@/lib/api";
import type { ConfigDirectoryDetection as DetectionResult } from "@/lib/api/settings";
import { Button } from "@/components/ui/button";

const LOCATION_SOURCE_KEYS = new Map([
  ["current", "settings.configDirectorySourceCurrent"],
  ["default", "settings.configDirectorySourceDefault"],
]);

const ENVIRONMENT_SOURCE_KEYS = new Map([
  ["process", "settings.configDirectorySourceProcess"],
  ["user", "settings.configDirectorySourceUser"],
  ["system", "settings.configDirectorySourceSystem"],
]);

type DetectionState =
  | { status: "loading" }
  | { status: "error" }
  | { status: "ready"; data: DetectionResult };

interface ConfigDirectoryDetectionProps {
  app: AppId;
  savedDirectory?: string;
  onAdopt?: (path: string) => void;
}

export function ConfigDirectoryDetection({
  app,
  savedDirectory,
  onAdopt,
}: ConfigDirectoryDetectionProps) {
  const { t } = useTranslation();
  const [state, setState] = useState<DetectionState>({ status: "loading" });
  const [revision, setRevision] = useState(0);

  const formatSource = (source: string) => {
    const locationKey = LOCATION_SOURCE_KEYS.get(source);
    if (locationKey) return t(locationKey);
    const separator = source.indexOf(":");
    const environmentKey = ENVIRONMENT_SOURCE_KEYS.get(
      source.slice(0, separator),
    );
    const name = source.slice(separator + 1);
    return environmentKey && name ? t(environmentKey, { name }) : source;
  };

  useEffect(() => {
    let active = true;
    setState({ status: "loading" });
    void settingsApi.detectConfigDirectories(app).then(
      (data) => {
        if (active) setState({ status: "ready", data });
      },
      () => {
        if (active) setState({ status: "error" });
      },
    );
    return () => {
      active = false;
    };
  }, [app, savedDirectory, revision]);

  return (
    <div className="min-w-0 space-y-2">
      <Button
        type="button"
        variant="outline"
        size="compact"
        disabled={state.status === "loading"}
        onClick={() => setRevision((previous) => previous + 1)}
      >
        {t("settings.detectConfigDirectories")}
      </Button>
      {onAdopt && (
        <p className="text-xs text-fg-2">
          {t("settings.detectedDirectorySaveHint")}
        </p>
      )}
      <div aria-live="polite" className="space-y-2 text-xs">
        {state.status === "loading" && (
          <p role="status">{t("common.loading")}</p>
        )}
        {state.status === "error" && (
          <p role="alert">{t("settings.configDirectoryDetectionFailed")}</p>
        )}
        {state.status === "ready" && (
          <>
            <div className="space-y-1">
              <p className="text-fg-2">
                {t("settings.currentConfigDirectory")}
              </p>
              <p className="break-all font-mono">{state.data.currentDir}</p>
            </div>
            {state.data.candidates.length === 0 && (
              <p className="text-fg-2">
                {t("settings.noConfigDirectoriesDetected")}
              </p>
            )}
            <ul className="space-y-2">
              {state.data.candidates.map((candidate) => (
                <li
                  key={candidate.path}
                  className="space-y-1 rounded-control border border-border p-2"
                >
                  <p className="break-all font-mono">{candidate.path}</p>
                  <p className="break-all text-fg-2">
                    {candidate.sources.map(formatSource).join(", ")}
                  </p>
                  {!candidate.exists && (
                    <p className="text-fg-2">
                      {t("settings.configDirectoryMissing")}
                    </p>
                  )}
                  {onAdopt && (
                    <Button
                      type="button"
                      variant="outline"
                      size="compact"
                      aria-label={`${t("settings.adoptConfigDirectory")} ${candidate.path}`}
                      disabled={!candidate.exists}
                      onClick={() => onAdopt(candidate.path)}
                    >
                      {t("settings.adoptConfigDirectory")}
                    </Button>
                  )}
                </li>
              ))}
            </ul>
          </>
        )}
      </div>
    </div>
  );
}
