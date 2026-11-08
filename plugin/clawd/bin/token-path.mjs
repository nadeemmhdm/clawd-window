import os from "node:os";
import path from "node:path";

export function apiTokenPath(env = process.env, platform = process.platform, home = os.homedir()) {
  const paths = platform === "win32" ? path.win32 : path.posix;
  let appData;
  if (platform === "win32") {
    appData = env.APPDATA || paths.join(home, "AppData", "Roaming");
  } else if (platform === "darwin") {
    appData = paths.join(home, "Library", "Application Support");
  } else {
    const xdg = env.XDG_DATA_HOME;
    appData = xdg && paths.isAbsolute(xdg) ? xdg : paths.join(home, ".local", "share");
  }
  return paths.join(appData, "com.clawd.pet", "clawd-api-token");
}
