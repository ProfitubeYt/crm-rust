// Ambient types for the Electron <webview> tag used by MiniAppHost, plus the
// electronAPI.miniApp bridge surface exposed by electron/preload.cjs.
//
// React's JSX.IntrinsicElements has no <webview>; without this the host TSX would
// not type-check under strict mode. Picked up ambiently via tsconfig `include: src`.

import type { DetailedHTMLProps, DOMAttributes, Ref, CSSProperties } from 'react';

interface WebviewHTMLAttributes<T> extends DOMAttributes<T> {
    src?: string;
    preload?: string;
    partition?: string;
    allowpopups?: boolean;
    disablewebsecurity?: boolean;
    useragent?: string;
    nodeintegration?: boolean;
    webpreferences?: string;
    style?: CSSProperties;
    className?: string;
    ref?: Ref<HTMLElement>;
}

declare global {
    namespace JSX {
        interface IntrinsicElements {
            webview: DetailedHTMLProps<WebviewHTMLAttributes<HTMLElement>, HTMLElement>;
        }
    }

    /** Snapshot of the desktop workbook watcher, as the main process reports it. */
    interface AnnotationWatcherState {
        configured: boolean;
        filePath?: string | null;
        fileName?: string | null;
        enabled?: boolean;
        lastSha256?: string | null;
        lastRunAt?: string | null;
    }

    interface Window {
        __TBS_NATIVE_SHELL__?: boolean;
        ipc?: { postMessage(message: string): void };
        __tbsShell?: {
            theme(value: 'light' | 'dark' | 'system'): void;
            toast(type: 'success' | 'error' | 'info' | 'warning', message: string): void;
        };
        __tbsMiniApp?: {
            event(raw: string): void;
            loaded(): void;
            navigated(): void;
        };
        electronAPI?: {
            platform?: string;
            isElectron?: boolean;
            miniApp?: {
                getPreloadPath: () => Promise<string>;
                openExternal: (url: string) => Promise<boolean>;
            };
            annotations?: {
                getState: () => Promise<AnnotationWatcherState>;
                pickFile: () => Promise<AnnotationWatcherState>;
                setEnabled: (enabled: boolean) => Promise<AnnotationWatcherState>;
                readFile: () => Promise<{ sha256: string; size: number; fileName: string; content: Uint8Array }>;
                markImported: (sha256: string) => Promise<AnnotationWatcherState>;
                onFileChanged: (cb: (payload: { path: string; sha256: string; size: number; detectedAt: string }) => void) => () => void;
            };
            [key: string]: unknown;
        };
    }
}

export {};
