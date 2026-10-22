// 应用导航标识；Qoder/DSH 使用各自原生 RPC，不传给文件投影型 provider 命令。
export type AppId =
  | "claude"
  | "claude-desktop"
  | "codex"
  | "gemini"
  | "grokbuild"
  | "opencode"
  | "openclaw"
  | "hermes"
  | "pi"
  | "mcode"
  | "qoder"
  | "dsh";
