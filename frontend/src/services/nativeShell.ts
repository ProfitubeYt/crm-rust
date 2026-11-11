/**
 * The native desktop client hosts the Telegram section in a WebView2 and loads
 * it from its own origin. Requests go through that origin, so credentials stay
 * in the native process; this file is the page's only line to it.
 *
 * The flag is set by an initialization script before any page code runs.
 */
export const isNativeShell = typeof window !== 'undefined' && Boolean(window.__TBS_NATIVE_SHELL__);

export interface ShellRect {
    x: number;
    y: number;
    width: number;
    height: number;
}

export type ShellMessage =
    | { type: 'ready' }
    | { type: 'open-external'; url: string }
    | { type: 'miniapp:open'; url: string; rect: ShellRect }
    | { type: 'miniapp:bounds'; rect: ShellRect }
    | { type: 'miniapp:visible'; visible: boolean }
    | { type: 'miniapp:send'; eventType: string; eventData: unknown }
    | { type: 'miniapp:close' };

export function postToShell(message: ShellMessage): void {
    window.ipc?.postMessage(JSON.stringify(message));
}
