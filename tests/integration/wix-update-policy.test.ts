import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "../..");
const source = readFileSync(
  resolve(root, "src-tauri/wix/per-user-main.wxs"),
  "utf8",
);
const rust = readFileSync(
  resolve(root, "src-tauri/src/commands/misc.rs"),
  "utf8",
);
const document = new DOMParser().parseFromString(source, "application/xml");

type InstallationScenario = {
  localAppData: string;
  selected?: string;
  ownMsi?: string;
  previousMsi?: string;
  previousNsis?: string;
};

// Evaluate the ordered WiX property assignments with synthetic registry and
// directory values. This checks the template policy, not Windows Installer.
function simulateInstallationDirectory(scenario: InstallationScenario): string {
  const properties: Record<string, string | undefined> = {
    INSTALLDIR: scenario.selected,
    LocalAppDataFolder: `${scenario.localAppData}\\`,
    CCOWNMSIINSTALLDIR: scenario.ownMsi,
    CCPREVMSIINSTALLDIR: scenario.previousMsi,
    CCPREVNSISINSTALLDIR: scenario.previousNsis,
  };
  const setters = document.querySelectorAll('SetProperty[Id="INSTALLDIR"]');
  for (const setter of setters) {
    const terms = setter.textContent?.trim().split(/\s+AND\s+/) ?? [];
    const matches = terms.every((term) => {
      if (term.startsWith("NOT ")) return !properties[term.slice(4)];
      return Boolean(properties[term]);
    });
    if (matches) {
      const value = setter.getAttribute("Value");
      if (!value) throw new Error("Unexpected INSTALLDIR assignment");
      properties.INSTALLDIR = value
        .replace(/\[([A-Za-z]+)\]/g, (_, property: string) => {
          const resolved = properties[property];
          if (!resolved) throw new Error(`Missing property ${property}`);
          return resolved;
        })
        .replace("{{product_name}}", "CC Switch");
    }
  }
  if (properties.INSTALLDIR) return properties.INSTALLDIR;

  throw new Error("No installation directory was selected");
}

describe("Windows installer update policy", () => {
  it("defaults to the current user's Programs directory without requiring D", () => {
    expect(document.querySelector("parsererror")).toBeNull();
    expect(source).not.toMatch(/CCSWITCHDAVAILABLE|CCSwitchDriveSearch|D:\\/i);
    const defaultSetter = document.querySelector(
      'SetProperty[Action="SetDefaultInstallDir"]',
    );
    expect(defaultSetter?.getAttribute("Value")).toBe(
      "[LocalAppDataFolder]Programs\\{{product_name}}",
    );
    expect(defaultSetter?.getAttribute("After")).toBe(
      "SetPreviousNsisInstallDir",
    );
    expect(defaultSetter?.getAttribute("Sequence")).toBe("first");
    expect(defaultSetter?.textContent?.trim()).toBe("NOT INSTALLDIR");
    const installDir = document.querySelector('Directory[Id="INSTALLDIR"]');
    expect(installDir?.parentElement?.getAttribute("Id")).toBe(
      "LocalAppDataFolder",
    );
    expect(installDir?.getAttribute("Name")).toBe("{{product_name}}");
    expect(
      simulateInstallationDirectory({
        localAppData: "C:\\Users\\Example\\AppData\\Local",
      }),
    ).toBe("C:\\Users\\Example\\AppData\\Local\\Programs\\CC Switch");
    expect(
      simulateInstallationDirectory({ localAppData: "E:\\UserProfile\\Local" }),
    ).toBe("E:\\UserProfile\\Local\\Programs\\CC Switch");
  });

  it("lets the user choose a C, D or other directory", () => {
    const folder = document.querySelector(
      'Dialog[Id="InstallDirOptionsDlg"] Control[Id="Folder"]',
    );
    expect(folder?.getAttribute("Type")).toBe("PathEdit");
    expect(folder?.getAttribute("Property")).toBe("WIXUI_INSTALLDIR");
    expect(folder?.getAttribute("Indirect")).toBe("yes");
    expect(
      document
        .querySelector('Property[Id="WIXUI_INSTALLDIR"]')
        ?.getAttribute("Value"),
    ).toBe("INSTALLDIR");
    expect(
      document
        .querySelector(
          'Publish[Dialog="InstallDirOptionsDlg"][Control="Next"][Event="SetTargetPath"]',
        )
        ?.getAttribute("Value"),
    ).toBe("[WIXUI_INSTALLDIR]");
    expect(
      document.querySelector(
        'Publish[Dialog="InstallDirOptionsDlg"][Control="ChangeFolder"][Value="BrowseDlg"]',
      ),
    ).not.toBeNull();
    for (const selected of [
      "C:\\Apps\\CC Switch",
      "D:\\Programs\\CCSwitch",
      "E:\\Personal Apps\\CC Switch",
    ]) {
      expect(
        simulateInstallationDirectory({
          localAppData: "C:\\Users\\Example\\AppData\\Local",
          selected,
          ownMsi: "D:\\Old\\CCSwitch",
        }),
      ).toBe(selected);
    }
  });

  it("prefers this fork's registered MSI, then the older MSI and NSIS paths", () => {
    for (const [property, action, after] of [
      ["CCOWNMSIINSTALLDIR", "SetOwnMsiInstallDir", "AppSearch"],
      [
        "CCPREVMSIINSTALLDIR",
        "SetPreviousMsiInstallDir",
        "SetOwnMsiInstallDir",
      ],
      [
        "CCPREVNSISINSTALLDIR",
        "SetPreviousNsisInstallDir",
        "SetPreviousMsiInstallDir",
      ],
    ]) {
      expect(
        document.querySelector(`Property[Id="${property}"] RegistrySearch`),
      ).not.toBeNull();
      const set = document.querySelector(`SetProperty[Action="${action}"]`);
      expect(set?.getAttribute("Id")).toBe("INSTALLDIR");
      expect(set?.getAttribute("Value")).toBe(`[${property}]`);
      expect(set?.getAttribute("After")).toBe(after);
      expect(set?.getAttribute("Sequence")).toBe("first");
      expect(set?.textContent?.trim()).toBe(`NOT INSTALLDIR AND ${property}`);
    }
    const localAppData = "C:\\Users\\Example\\AppData\\Local";
    expect(
      simulateInstallationDirectory({
        localAppData,
        ownMsi: "E:\\Current\\CCSwitch",
        previousMsi: "D:\\OldMsi\\CCSwitch",
        previousNsis: "C:\\OldNsis\\CCSwitch",
      }),
    ).toBe("E:\\Current\\CCSwitch");
    expect(
      simulateInstallationDirectory({
        localAppData,
        previousMsi: "D:\\OldMsi\\CCSwitch",
        previousNsis: "C:\\OldNsis\\CCSwitch",
      }),
    ).toBe("D:\\OldMsi\\CCSwitch");
    expect(
      simulateInstallationDirectory({
        localAppData,
        previousNsis: "C:\\OldNsis\\CCSwitch",
      }),
    ).toBe("C:\\OldNsis\\CCSwitch");
  });

  it("writes the same marker read by the application without deleting unrelated files", () => {
    const marker = document.querySelector(
      'RegistryValue[Root="HKCU"][Name="InstallDir"][Key]',
    );
    expect(marker?.getAttribute("Key")).toBe(
      "Software\\WQMYH\\MultiCompatCC\\Installer",
    );
    expect(marker?.getAttribute("Value")).toBe("[INSTALLDIR]");
    expect(rust).toContain(
      'open_subkey(r"Software\\WQMYH\\MultiCompatCC\\Installer")',
    );
    expect(
      document.querySelector('RemoveFile[Name="portable.ini"]'),
    ).toBeNull();
    expect(
      document.querySelector('RemoveFile[Id="RemoveUserProgramsFiles"]'),
    ).toBeNull();
    expect(document.querySelectorAll("RemoveFile")).toHaveLength(0);
    expect(
      Array.from(document.querySelectorAll("RemoveFolder")).map((folder) => ({
        id: folder.getAttribute("Id"),
        on: folder.getAttribute("On"),
      })),
    ).toEqual([
      { id: "INSTALLDIR", on: "uninstall" },
      { id: "ApplicationProgramsFolder", on: "uninstall" },
    ]);
    expect(
      Array.from(document.querySelectorAll("Directory")).some((directory) =>
        ["data", "cache"].includes(
          directory.getAttribute("Name")?.toLowerCase() ?? "",
        ),
      ),
    ).toBe(false);
    expect(
      document
        .querySelector('Property[Id="ARPURLUPDATEINFO"]')
        ?.getAttribute("Value"),
    ).toBe("https://github.com/WQMYH/multi-compat-cc/releases");
  });
});
