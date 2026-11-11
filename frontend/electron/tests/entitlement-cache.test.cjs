/**
 * The cached entitlement.
 *
 * Its job is to keep a laptop that lost the network for a morning working,
 * without becoming a way to work indefinitely with the cable unplugged. Both
 * halves of that are tested here, because getting either wrong is invisible
 * until it locks out a paying customer or fails to lock out an expired one.
 */
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const cache = require('../entitlement-cache.cjs');

function fakeApp() {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'entitlement-'));
    return { getPath: () => dir, dir };
}

const DAY = 86400000;

test('a missing cache is not an error, just nothing to go on', () => {
    const app = fakeApp();
    assert.equal(cache.read(app), null);
    assert.deepEqual(cache.evaluate(null), { usable: false, reason: 'no_cache' });
});

test('a corrupt cache reads as absent rather than throwing', () => {
    const app = fakeApp();
    fs.writeFileSync(cache.statePath(app), 'не json', 'utf8');
    assert.equal(cache.read(app), null);
});

test('what the server said survives a round trip', () => {
    const app = fakeApp();
    cache.write(app, {
        ok: true,
        expired: false,
        allowed_until: '2026-12-31T23:59:59+00:00',
        note: 'оплачено до конца года',
        username: 'client',
    });
    const record = cache.read(app);
    assert.equal(record.ok, true);
    assert.equal(record.allowedUntil, '2026-12-31T23:59:59+00:00');
    assert.equal(record.note, 'оплачено до конца года');
    assert.ok(record.checkedAt);
});

test('a fresh cache lets an offline launch through', () => {
    const now = new Date('2026-06-10T12:00:00Z');
    const verdict = cache.evaluate({
        ok: true,
        allowedUntil: '2026-12-31T23:59:59Z',
        checkedAt: new Date(now.getTime() - 2 * DAY).toISOString(),
    }, now);
    assert.equal(verdict.usable, true);
    assert.equal(verdict.allowed, true);
});

test('the grace window closes, so the cache is not an offline licence', () => {
    const now = new Date('2026-06-10T12:00:00Z');
    const verdict = cache.evaluate({
        ok: true,
        allowedUntil: '2026-12-31T23:59:59Z',
        checkedAt: new Date(now.getTime() - (cache.OFFLINE_GRACE_DAYS + 1) * DAY).toISOString(),
    }, now);
    assert.equal(verdict.usable, false);
    assert.equal(verdict.reason, 'grace_expired');
});

test('a cache cannot outlive the entitlement it describes', () => {
    // Written yesterday, but the date it carries has since passed: turning the
    // clock forward must not be survivable by having checked in recently.
    const now = new Date('2026-06-10T12:00:00Z');
    const verdict = cache.evaluate({
        ok: true,
        allowedUntil: '2026-06-01T23:59:59Z',
        checkedAt: new Date(now.getTime() - DAY).toISOString(),
    }, now);
    assert.equal(verdict.usable, true);
    assert.equal(verdict.allowed, false);
    assert.equal(verdict.reason, 'expired');
});

test('a clock rolled backwards is distrusted, not rewarded', () => {
    const now = new Date('2026-06-10T12:00:00Z');
    const verdict = cache.evaluate({
        ok: true,
        allowedUntil: '2026-12-31T23:59:59Z',
        checkedAt: new Date(now.getTime() + 30 * DAY).toISOString(),
    }, now);
    assert.equal(verdict.usable, false);
    assert.equal(verdict.reason, 'clock_skew');
});

test('no date at all means unlimited', () => {
    const now = new Date('2026-06-10T12:00:00Z');
    const verdict = cache.evaluate({
        ok: true,
        allowedUntil: null,
        checkedAt: new Date(now.getTime() - DAY).toISOString(),
    }, now);
    assert.equal(verdict.allowed, true);
});

test('clearing removes the file', () => {
    const app = fakeApp();
    cache.write(app, { ok: true });
    assert.ok(fs.existsSync(cache.statePath(app)));
    cache.clear(app);
    assert.equal(fs.existsSync(cache.statePath(app)), false);
    // And clearing again is not an error.
    cache.clear(app);
});
