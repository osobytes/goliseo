// Ported from game/build_info.lua.

export interface BuildInfo {
  readonly name: string;
  readonly version: string;
  readonly channel: "development" | "release";
  readonly source_url?: string;
}

export const buildInfo: BuildInfo = {
  name: "GOLISEO",
  version: "0.1.0-dev",
  channel: "development",
};
