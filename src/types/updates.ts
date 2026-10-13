export interface UpdateRelease {
  version: string;
  publishedAt: string;
  notes: { "zh-CN": string[]; en: string[] };
  downloads: { github: string; quark: string | null; quarkCode: string | null };
}

export interface UpdateCheckResult {
  currentVersion: string;
  status: "updateAvailable" | "upToDate" | "platformUnavailable";
  release: UpdateRelease | null;
  checkedAt: string;
  fromCache: boolean;
}
