/**
 * Background watcher for the client's bookkeeping workbook.
 *
 * The customer keeps annotating in Excel. This watches the file they point at
 * and tells the renderer whenever it changes, so their notes flow into the
 * program without anyone remembering to upload anything.
 *
 * The main process deliberately does NOT make the HTTP request. The access
 * token lives in the renderer, and `X-System-Access` is attached by the header
 * broker only for requests the trusted renderer makes. Main detects the change
 * and hands over the bytes; the renderer posts them through the normal API
 * client. That keeps authentication in exactly one place.
 *
 * Windows specifics this has to survive:
 *  - Excel writes a temp file and renames it over the original, so a single
 *    save fires several rename/change events and can detach a file watcher.
 *    Hence: watch the *directory*, plus a stat poll as a safety net.
 *  - Excel holds an exclusive lock while the workbook is open, so reading has
 *    to retry rather than fail.
 */
const crypto = require('crypto');
const fs = require('fs');
const fsp = require('fs/promises');
const path = require('path');

const STATE_FILE = 'annotation-watcher.json';
const DEBOUNCE_MS = 3000;
const POLL_INTERVAL_MS = 5000;
const LOCK_RETRY_MS = 750;
const LOCK_MAX_WAIT_MS = 30000;
const MAX_FILE_BYTES = 25 * 1024 * 1024;

let ctx = null;

function statePath(app) {
    return path.join(app.getPath('userData'), STATE_FILE);
}

function readState(app) {
    try {
        const raw = fs.readFileSync(statePath(app), 'utf8');
        const parsed = JSON.parse(raw);
        return {
            filePath: typeof parsed.filePath === 'string' ? parsed.filePath : null,
            enabled: Boolean(parsed.enabled),
            lastSha256: typeof parsed.lastSha256 === 'string' ? parsed.lastSha256 : null,
            lastRunAt: typeof parsed.lastRunAt === 'string' ? parsed.lastRunAt : null,
        };
    } catch {
        // Missing or corrupt state is not an error: start fresh.
        return { filePath: null, enabled: false, lastSha256: null, lastRunAt: null };
    }
}

function writeState(app, state) {
    try {
        fs.writeFileSync(statePath(app), JSON.stringify(state, null, 2), 'utf8');
    } catch (error) {
        // Losing the state only costs one redundant upload; the server
        // deduplicates by file digest anyway.
        console.error('[annotation-watcher] failed to persist state:', error.message);
    }
}

/**
 * Read the workbook, waiting out Excel's exclusive lock.
 * Returns { content, sha256, size } or throws after LOCK_MAX_WAIT_MS.
 */
async function readWorkbook(filePath) {
    const deadline = Date.now() + LOCK_MAX_WAIT_MS;
    let lastError = null;
    while (Date.now() < deadline) {
        try {
            const stat = await fsp.stat(filePath);
            if (stat.size > MAX_FILE_BYTES) {
                throw new Error('file_too_large');
            }
            const handle = await fsp.open(filePath, 'r');
            try {
                const content = await handle.readFile();
                return {
                    content,
                    size: content.length,
                    sha256: crypto.createHash('sha256').update(content).digest('hex'),
                };
            } finally {
                await handle.close();
            }
        } catch (error) {
            if (error.message === 'file_too_large') throw error;
            lastError = error;
            await new Promise((resolve) => setTimeout(resolve, LOCK_RETRY_MS));
        }
    }
    throw lastError || new Error('workbook_unreadable');
}

function clearWatchers() {
    if (!ctx) return;
    if (ctx.watcher) {
        try { ctx.watcher.close(); } catch { /* already closed */ }
        ctx.watcher = null;
    }
    if (ctx.pollTimer) {
        clearInterval(ctx.pollTimer);
        ctx.pollTimer = null;
    }
    if (ctx.debounceTimer) {
        clearTimeout(ctx.debounceTimer);
        ctx.debounceTimer = null;
    }
}

async function notifyIfChanged() {
    if (!ctx || !ctx.state.enabled || !ctx.state.filePath) return;
    try {
        const { sha256, size } = await readWorkbook(ctx.state.filePath);
        if (sha256 === ctx.state.lastSha256) return; // nothing new
        ctx.pendingSha256 = sha256;
        if (ctx.mainWindow() && !ctx.mainWindow().isDestroyed()) {
            ctx.mainWindow().webContents.send('annotations:file-changed', {
                path: ctx.state.filePath,
                sha256,
                size,
                detectedAt: new Date().toISOString(),
            });
        }
    } catch (error) {
        // A missing file is expected mid-save: Excel writes a temp file and
        // renames it over the original, so the path briefly does not exist.
        if (error && error.code !== 'ENOENT') {
            console.error('[annotation-watcher] read failed:', error.message);
        }
    }
}

function scheduleCheck() {
    if (!ctx) return;
    if (ctx.debounceTimer) clearTimeout(ctx.debounceTimer);
    // One Excel save produces a burst of events; act once it settles.
    ctx.debounceTimer = setTimeout(() => { void notifyIfChanged(); }, DEBOUNCE_MS);
}

function startWatching() {
    if (!ctx) return;
    clearWatchers();
    const { filePath, enabled } = ctx.state;
    if (!enabled || !filePath) return;

    const directory = path.dirname(filePath);
    const baseName = path.basename(filePath);

    try {
        // Watch the directory, not the file: Excel replaces the inode on save
        // and a file-level watcher silently stops firing afterwards.
        ctx.watcher = fs.watch(directory, (_eventType, changed) => {
            if (!changed || path.basename(String(changed)) !== baseName) return;
            scheduleCheck();
        });
    } catch (error) {
        console.error('[annotation-watcher] fs.watch failed:', error.message);
    }

    // Safety net for the cases where fs.watch drops the subscription entirely.
    let lastSignature = '';
    ctx.pollTimer = setInterval(async () => {
        try {
            const stat = await fsp.stat(filePath);
            const signature = `${stat.mtimeMs}:${stat.size}`;
            if (lastSignature && signature !== lastSignature) scheduleCheck();
            lastSignature = signature;
        } catch {
            // File temporarily gone (Excel's rename dance) or removed.
        }
    }, POLL_INTERVAL_MS);
}

function publicState() {
    if (!ctx) return { configured: false };
    return {
        configured: Boolean(ctx.state.filePath),
        filePath: ctx.state.filePath,
        fileName: ctx.state.filePath ? path.basename(ctx.state.filePath) : null,
        enabled: Boolean(ctx.state.enabled),
        lastSha256: ctx.state.lastSha256,
        lastRunAt: ctx.state.lastRunAt,
    };
}

/**
 * Register the watcher's IPC surface.
 *
 * @param {object} deps
 * @param {import('electron').App} deps.app
 * @param {import('electron').IpcMain} deps.ipcMain
 * @param {import('electron').Dialog} deps.dialog
 * @param {() => import('electron').BrowserWindow | null} deps.mainWindow
 * @param {(event: unknown) => void} deps.requireTrustedRenderer
 */
function setupAnnotationWatcher({ app, ipcMain, dialog, mainWindow, requireTrustedRenderer }) {
    ctx = {
        app,
        mainWindow,
        state: readState(app),
        watcher: null,
        pollTimer: null,
        debounceTimer: null,
        pendingSha256: null,
    };

    ipcMain.handle('annotations:get-state', (event) => {
        requireTrustedRenderer(event);
        return publicState();
    });

    ipcMain.handle('annotations:pick-file', async (event) => {
        requireTrustedRenderer(event);
        const result = await dialog.showOpenDialog(mainWindow() || undefined, {
            title: 'Выберите книгу учёта',
            filters: [{ name: 'Excel', extensions: ['xlsx', 'xlsm'] }],
            properties: ['openFile'],
        });
        if (result.canceled || !result.filePaths.length) return publicState();

        ctx.state.filePath = result.filePaths[0];
        // A newly chosen file has never been imported from.
        ctx.state.lastSha256 = null;
        ctx.state.enabled = true;
        writeState(app, ctx.state);
        startWatching();
        void notifyIfChanged();
        return publicState();
    });

    ipcMain.handle('annotations:set-enabled', (event, enabled) => {
        requireTrustedRenderer(event);
        ctx.state.enabled = Boolean(enabled);
        writeState(app, ctx.state);
        if (ctx.state.enabled) startWatching();
        else clearWatchers();
        return publicState();
    });

    ipcMain.handle('annotations:read-file', async (event) => {
        requireTrustedRenderer(event);
        if (!ctx.state.filePath) throw new Error('no_file_selected');
        const { content, sha256, size } = await readWorkbook(ctx.state.filePath);
        return {
            sha256,
            size,
            fileName: path.basename(ctx.state.filePath),
            // Uint8Array survives the structured clone to the renderer intact.
            content: new Uint8Array(content),
        };
    });

    // Called by the renderer once the server has accepted an import, so a
    // repeated save of the same content does not upload again.
    ipcMain.handle('annotations:mark-imported', (event, sha256) => {
        requireTrustedRenderer(event);
        if (typeof sha256 === 'string' && sha256) {
            ctx.state.lastSha256 = sha256;
            ctx.state.lastRunAt = new Date().toISOString();
            writeState(app, ctx.state);
        }
        return publicState();
    });

    startWatching();
}

function stopAnnotationWatcher() {
    clearWatchers();
    ctx = null;
}

module.exports = {
    setupAnnotationWatcher,
    stopAnnotationWatcher,
    // exported for tests
    _internals: { readState, writeState, publicState },
};
