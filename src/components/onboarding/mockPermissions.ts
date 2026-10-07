// Browser-mock side of macOS folder access: Full Disk Access starts "denied"
// and flips to "granted" a few seconds after "Open Settings" (as if the user
// turned the switch on). `?fda=granted` in the URL starts granted.
import type { Api } from "../../api";
import type { Access, PermissionsStatus } from "../../types";

type PermissionsApi = Pick<Api, "permissionsStatus" | "openPrivacySettings">;

/** How long the pretend user takes in System Settings. */
export const MOCK_GRANT_AFTER_MS = 4000;

export function mockStatus(fda: Access): PermissionsStatus {
  const folders: Access = fda === "granted" ? "granted" : "unknown";
  return { applies: true, fullDiskAccess: fda, desktop: folders, documents: folders, downloads: folders };
}

export function createPermissionsMock(now: () => number = Date.now): PermissionsApi {
  const start = typeof location !== "undefined" && new URLSearchParams(location.search).get("fda") === "granted";
  let grantedAt: number | null = start ? 0 : null;
  return {
    async permissionsStatus() {
      return mockStatus(grantedAt !== null && now() >= grantedAt ? "granted" : "denied");
    },
    async openPrivacySettings(kind) {
      console.info(`[mock] open_privacy_settings(${kind}) → granting in ${MOCK_GRANT_AFTER_MS / 1000} s`);
      if (kind === "fullDiskAccess" && grantedAt === null) grantedAt = now() + MOCK_GRANT_AFTER_MS;
    },
  };
}
