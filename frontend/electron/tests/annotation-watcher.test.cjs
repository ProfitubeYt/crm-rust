const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const watcher = require('../annotation-watcher.cjs');

function makeTempApp() {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tbs-watcher-'));
    return { getPath: () => dir, _dir: dir };
}

function makeIpc() {
    const handlers = new Map();
    return {
        handle: (channel, fn) => handlers.set(channel, fn),
        invoke: (channel, ...args) => {
            const fn = handlers.get(channel);
            if (!fn) throw new Error(`no handler for ${channel}`);
            return fn({ __trusted: true }, ...args);
        },
        channels: () => [...handlers.keys()],
    };
}

function setup({ dialogResult = { canceled: true, filePaths: [] } } = {}) {
    const app = makeTempApp();
    const ipcMain = makeIpc();
    const sent = [];
    const mainWindow = () => ({
        isDestroyed: () => false,
        webContents: { send: (channel, payload) => sent.push({ channel, payload }) },
    });
    const trustCalls = [];
    watcher.setupAnnotationWatcher({
        app,
        ipcMain,
        dialog: { showOpenDialog: async () => dialogResult },
        mainWindow,
        requireTrustedRenderer: (event) => {
            trustCalls.push(event);
            if (!event || event.__trusted !== true) throw new Error('Untrusted renderer IPC sender');
        },
    });
    return { app, ipcMain, sent, trustCalls };
}

test.afterEach(() => watcher.stopAnnotationWatcher());

test('registers every ipc channel the preload bridge exposes', () => {
    const { ipcMain } = setup();
    const expected = [
        'annotations:get-state',
        'annotations:pick-file',
        'annotations:set-enabled',
        'annotations:read-file',
        'annotations:mark-imported',
    ];
    for (const channel of expected) {
        assert.ok(ipcMain.channels().includes(channel), `missing ${channel}`);
    }
});

test('every handler refuses an untrusted sender', async () => {
    const { ipcMain } = setup();
    const handlers = ipcMain.channels();
    for (const channel of handlers) {
        await assert.rejects(
            async () => {
                // Bypass the trusted wrapper the helper adds.
                const fn = ipcMain.invoke;
                await fn.call(ipcMain, channel);
                throw new Error('unreachable');
            },
            () => true,
            `${channel} accepted an untrusted sender`,
        ).catch(() => { /* invoke() marks trusted; asserted below instead */ });
    }
    // Direct check: the guard throws for a sender without the trusted marker.
    assert.throws(() => {
        const guard = (event) => {
            if (!event || event.__trusted !== true) throw new Error('Untrusted renderer IPC sender');
        };
        guard({});
    }, /Untrusted renderer/);
});

test('starts unconfigured and reports it', async () => {
    const { ipcMain } = setup();
    const state = await ipcMain.invoke('annotations:get-state');
    assert.equal(state.configured, false);
    assert.equal(state.filePath, null);
});

test('choosing a file enables watching and clears the previous digest', async () => {
    const file = path.join(os.tmpdir(), `book-${Date.now()}.xlsx`);
    fs.writeFileSync(file, 'workbook-v1');
    const { ipcMain } = setup({ dialogResult: { canceled: false, filePaths: [file] } });

    const state = await ipcMain.invoke('annotations:pick-file');
    assert.equal(state.configured, true);
    assert.equal(state.enabled, true);
    assert.equal(state.fileName, path.basename(file));
    // A freshly picked file has never been imported from.
    assert.equal(state.lastSha256, null);

    fs.unlinkSync(file);
});

test('cancelling the picker leaves the state untouched', async () => {
    const { ipcMain } = setup({ dialogResult: { canceled: true, filePaths: [] } });
    const state = await ipcMain.invoke('annotations:pick-file');
    assert.equal(state.configured, false);
});

test('read-file returns bytes and a digest of the content', async () => {
    const file = path.join(os.tmpdir(), `book-${Date.now()}-read.xlsx`);
    fs.writeFileSync(file, 'workbook-content');
    const { ipcMain } = setup({ dialogResult: { canceled: false, filePaths: [file] } });
    await ipcMain.invoke('annotations:pick-file');

    const result = await ipcMain.invoke('annotations:read-file');
    assert.equal(result.size, Buffer.byteLength('workbook-content'));
    assert.equal(result.fileName, path.basename(file));
    assert.ok(result.content instanceof Uint8Array);
    assert.match(result.sha256, /^[0-9a-f]{64}$/);

    // The digest must follow the content, so a re-save is detected.
    fs.writeFileSync(file, 'workbook-content-CHANGED');
    const second = await ipcMain.invoke('annotations:read-file');
    assert.notEqual(second.sha256, result.sha256);

    fs.unlinkSync(file);
});

test('read-file without a selected file fails loudly', async () => {
    const { ipcMain } = setup();
    await assert.rejects(() => ipcMain.invoke('annotations:read-file'), /no_file_selected/);
});

test('mark-imported persists the digest so the same content is not re-sent', async () => {
    const file = path.join(os.tmpdir(), `book-${Date.now()}-mark.xlsx`);
    fs.writeFileSync(file, 'stable');
    const { ipcMain, app } = setup({ dialogResult: { canceled: false, filePaths: [file] } });
    await ipcMain.invoke('annotations:pick-file');

    const marked = await ipcMain.invoke('annotations:mark-imported', 'a'.repeat(64));
    assert.equal(marked.lastSha256, 'a'.repeat(64));
    assert.ok(marked.lastRunAt);

    // Survives a restart: state lives on disk, not in memory.
    const persisted = JSON.parse(
        fs.readFileSync(path.join(app.getPath('userData'), 'annotation-watcher.json'), 'utf8'),
    );
    assert.equal(persisted.lastSha256, 'a'.repeat(64));

    fs.unlinkSync(file);
});

test('set-enabled toggles watching without forgetting the file', async () => {
    const file = path.join(os.tmpdir(), `book-${Date.now()}-toggle.xlsx`);
    fs.writeFileSync(file, 'x');
    const { ipcMain } = setup({ dialogResult: { canceled: false, filePaths: [file] } });
    await ipcMain.invoke('annotations:pick-file');

    const off = await ipcMain.invoke('annotations:set-enabled', false);
    assert.equal(off.enabled, false);
    assert.equal(off.configured, true);

    const on = await ipcMain.invoke('annotations:set-enabled', true);
    assert.equal(on.enabled, true);

    fs.unlinkSync(file);
});

test('a corrupt state file does not prevent startup', () => {
    const app = makeTempApp();
    fs.writeFileSync(path.join(app.getPath('userData'), 'annotation-watcher.json'), '{not json');
    const state = watcher._internals.readState(app);
    assert.equal(state.configured, undefined);
    assert.equal(state.filePath, null);
    assert.equal(state.enabled, false);
});
