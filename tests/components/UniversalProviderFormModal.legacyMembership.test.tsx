import type { ReactNode } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { UniversalProvider } from "@/types";
import { UniversalProviderFormModal } from "@/components/universal/UniversalProviderFormModal";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) =>
      options?.defaultValue ?? key,
  }),
}));
vi.mock("@/hooks/useDarkMode", () => ({ useDarkMode: () => false }));
vi.mock("@/components/common/FullScreenPanel", () => ({
  FullScreenPanel: ({
    children,
    footer,
  }: {
    children: ReactNode;
    footer: ReactNode;
  }) => (
    <>
      {children}
      {footer}
    </>
  ),
}));
vi.mock("@/components/JsonEditor", () => ({ default: () => null }));
vi.mock("@/components/ConfirmDialog", () => ({
  ConfirmDialog: ({
    isOpen,
    onConfirm,
  }: {
    isOpen: boolean;
    onConfirm: () => void;
  }) => (isOpen ? <button onClick={onConfirm}>confirm-sync</button> : null),
}));

describe("DSH-only universal provider editing", () => {
  it.each([false, true])(
    "preserves legacy memberships while saving (sync=%s)",
    (sync) => {
      const provider: UniversalProvider = {
        id: "saved",
        name: "Saved",
        providerType: "custom",
        baseUrl: "https://fixture.invalid/v1",
        apiKey: "synthetic-key",
        apps: {
          claude: false,
          codex: false,
          gemini: false,
          qoder: true,
          dsh: true,
        },
        models: {
          qoder: { model: "legacy-qoder" },
          dsh: { model: "native-dsh" },
        },
      };
      const saved = vi.fn();
      render(
        <UniversalProviderFormModal
          isOpen
          onClose={vi.fn()}
          editingProvider={provider}
          onSave={saved}
          onSaveAndSync={sync ? saved : undefined}
        />,
      );
      expect(screen.queryByText("Qoder CN")).toBeNull();
      fireEvent.change(screen.getByLabelText("名称"), {
        target: { value: "Renamed" },
      });
      fireEvent.click(
        screen.getByRole("button", { name: sync ? "保存并同步" : "添加" }),
      );
      if (sync)
        fireEvent.click(screen.getByRole("button", { name: "confirm-sync" }));
      expect(saved).toHaveBeenCalledOnce();
      const result = saved.mock.calls[0][0] as UniversalProvider;
      expect(result.name).toBe("Renamed");
      expect(result.apps).toEqual(provider.apps);
      expect(result.models).toEqual(provider.models);
    },
  );
});
