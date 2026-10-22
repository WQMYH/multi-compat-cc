import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ManagedNativeApplication } from "@/components/apps/ManagedNativeApplication";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  catalog: vi.fn(),
  add: vi.fn(),
  update: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({ invalidateQueries: vi.fn() }),
  useQuery: () => ({
    data: {
      cliPath: null,
      dataDir: null,
      detectionError: "ambiguous installation",
      provider: null,
      readonlyPreview: true,
    },
  }),
}));
vi.mock("@/lib/query", () => ({ useProvidersQuery: mocks.catalog }));
vi.mock("@/hooks/useProviderActions", () => ({
  useProviderActions: () => ({
    addProvider: mocks.add,
    updateProvider: mocks.update,
  }),
}));
vi.mock("@/lib/toast", () => ({
  toast: { info: vi.fn(), error: vi.fn(), success: vi.fn() },
}));
vi.mock("@/lib/api", () => ({ settingsApi: { openExternal: vi.fn() } }));
vi.mock("@/components/providers/ProviderList", () => ({
  ProviderList: ({ providers, onSwitch, onEdit }: any) => (
    <>
      {Object.values(providers).map((provider: any) => (
        <div key={provider.id}>
          {provider.name}
          <button onClick={() => onSwitch(provider)}>选择托管供应商</button>
          <button onClick={() => onEdit(provider)}>原编辑窗口</button>
        </div>
      ))}
    </>
  ),
}));
vi.mock("@/components/providers/AddProviderDialog", () => ({
  AddProviderDialog: ({ open, appId }: any) =>
    open ? <div>原新增窗口：{appId}</div> : null,
}));
vi.mock("@/components/providers/EditProviderDialog", () => ({
  EditProviderDialog: ({ open, provider }: any) =>
    open ? <div>原编辑数据：{provider.name}</div> : null,
}));

describe("CC managed native application surface", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.catalog.mockReturnValue({
      data: {
        providers: {
          managed: {
            id: "managed",
            name: "Existing CC account provider",
            settingsConfig: {},
          },
        },
      },
      isLoading: false,
    });
  });

  it.each(["codex", "claude"] as const)(
    "uses the %s catalog and the shared page header's dialog state without an extra page",
    (source) => {
      const props = {
        target: "dsh" as const,
        source,
        adding: false,
        onAddingChange: vi.fn(),
        onManageSource: vi.fn(),
      };
      const { rerender } = render(<ManagedNativeApplication {...props} />);
      expect(mocks.catalog).toHaveBeenCalledWith(source);
      expect(screen.getByText("Existing CC account provider")).toBeTruthy();
      expect(screen.queryByText("使用 CC 已托管的供应商和账号")).toBeNull();
      expect(screen.queryByRole("button", { name: "添加供应商" })).toBeNull();
      expect(screen.queryByRole("tablist")).toBeNull();
      expect(screen.queryByText(/数据目录：/)).toBeNull();
      expect(
        screen.queryByRole("button", { name: "安装／升级／删除应用" }),
      ).toBeNull();
      expect(screen.queryAllByRole("textbox")).toHaveLength(0);
      rerender(<ManagedNativeApplication {...props} adding />);
      expect(screen.getByText(`原新增窗口：${source}`)).toBeTruthy();
      fireEvent.click(screen.getByText("原编辑窗口"));
      expect(
        screen.getByText("原编辑数据：Existing CC account provider"),
      ).toBeTruthy();
      expect(mocks.invoke).not.toHaveBeenCalled();
    },
  );

  it("does not switch accounts, bind a provider or invoke a model during readonly initial acceptance", async () => {
    render(
      <ManagedNativeApplication
        target="dsh"
        source="codex"
        adding={false}
        onAddingChange={vi.fn()}
        onManageSource={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByText("选择托管供应商"));
    await waitFor(() => expect(mocks.invoke).not.toHaveBeenCalled());
    expect(mocks.add).not.toHaveBeenCalled();
    expect(mocks.update).not.toHaveBeenCalled();
  });
});
