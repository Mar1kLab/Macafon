// Link downloads (SND-009): what the UI checks before asking the daemon, and how to install
// the tools it needs.

/** An http(s) link with a host; the daemon checks again before running yt-dlp. */
export function isLink(text: string): boolean {
  try {
    const url = new URL(text.trim());
    return (url.protocol === 'http:' || url.protocol === 'https:') && url.hostname !== '';
  } catch {
    return false;
  }
}

/** The shell command that installs yt-dlp (winget on Windows, pipx on Linux). */
export function installCommand(windows: boolean): string {
  return windows ? 'winget install yt-dlp.yt-dlp' : 'pipx install yt-dlp';
}
