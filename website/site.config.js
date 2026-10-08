// Where the source lives and where the site is published. This is the only place
// the GitHub owner and repository name appear; pages use %REPO_URL% and
// %PAGES_URL%, which vite.config.js fills in at build time.
//
// The deployed base path (/pitwall/ on GitHub Pages) is not set here: the Pages
// workflow passes it from the repository name (`pnpm build --base /<repo>/`).
export const OWNER = "AleksandreJavakhishvili";
export const REPO = "pitwall";

export const REPO_URL = `https://github.com/${OWNER}/${REPO}`;
export const PAGES_URL = `https://${OWNER.toLowerCase()}.github.io/${REPO}/`;
export const RELEASES_URL = `${REPO_URL}/releases/latest`;
// The download page reads the latest release (version, date, asset URLs) from here
// at runtime and falls back to RELEASES_URL when it can't.
export const RELEASE_API_URL = `https://api.github.com/repos/${OWNER}/${REPO}/releases/latest`;
