/**
 * Remembering how long this account may use the app.
 *
 * The customer installs once and works daily; asking them to sign in again
 * because a laptop was offline for a morning is the wrong trade. So the answer
 * from `GET /api/auth/entitlement` is kept on disk, and a launch that cannot
 * reach the server falls back to it.
 *
 * The grace period is what makes that safe in both directions. A cached "you
 * may work until the 30th" is honoured while offline, but only for a few days —
 * long enough to survive a trip or an outage, short enough that the cache is
 * not a way to keep working indefinitely with the network unplugged.
 *
 * Nothing here decides anything on its own. It stores what the server said and
 * reports whether that answer is still worth believing; the renderer does the
 * gating, and the server is the one that actually refuses requests.
 */
const fs = require('fs');
const path = require('path');

const STATE_FILE = 'entitlement.json';
// How long a cached answer stays usable without the server confirming it.
const OFFLINE_GRACE_DAYS = 7;

function statePath(app) {
    return path.join(app.getPath('userData'), STATE_FILE);
}

function read(app) {
    try {
        const parsed = JSON.parse(fs.readFileSync(statePath(app), 'utf8'));
        if (!parsed || typeof parsed !== 'object') return null;
        return {
            ok: Boolean(parsed.ok),
            expired: Boolean(parsed.expired),
            allowedUntil: typeof parsed.allowedUntil === 'string' ? parsed.allowedUntil : null,
            note: typeof parsed.note === 'string' ? parsed.note : null,
            username: typeof parsed.username === 'string' ? parsed.username : null,
            checkedAt: typeof parsed.checkedAt === 'string' ? parsed.checkedAt : null,
        };
    } catch {
        // Missing or corrupt: no cached answer, which is not an error — the app
        // simply has to ask the server before deciding anything.
        return null;
    }
}

function write(app, entitlement) {
    const record = {
        ok: Boolean(entitlement && entitlement.ok),
        expired: Boolean(entitlement && entitlement.expired),
        allowedUntil: (entitlement && entitlement.allowed_until) || null,
        note: (entitlement && entitlement.note) || null,
        username: (entitlement && entitlement.username) || null,
        checkedAt: new Date().toISOString(),
    };
    try {
        fs.writeFileSync(statePath(app), JSON.stringify(record, null, 2), 'utf8');
    } catch (error) {
        // Losing the cache costs one blocked offline launch, not data.
        console.error('[entitlement] failed to persist:', error.message);
    }
    return record;
}

function clear(app) {
    try {
        fs.unlinkSync(statePath(app));
    } catch {
        // Already gone.
    }
}

/**
 * What a cached answer is worth right now.
 *
 * `usable` is false once the grace window closes or the stored date itself has
 * passed — a cache cannot outlive the entitlement it describes.
 */
function evaluate(record, now = new Date()) {
    if (!record || !record.checkedAt) {
        return { usable: false, reason: 'no_cache' };
    }

    const checkedAt = new Date(record.checkedAt);
    if (Number.isNaN(checkedAt.getTime())) {
        return { usable: false, reason: 'no_cache' };
    }

    const ageDays = (now.getTime() - checkedAt.getTime()) / 86400000;
    if (ageDays > OFFLINE_GRACE_DAYS) {
        return { usable: false, reason: 'grace_expired', ageDays };
    }
    // A clock rolled far backwards makes the cache look freshly written; treat
    // that as untrustworthy rather than as a free extension.
    if (ageDays < -1) {
        return { usable: false, reason: 'clock_skew', ageDays };
    }

    if (record.allowedUntil) {
        const until = new Date(record.allowedUntil);
        if (!Number.isNaN(until.getTime()) && until <= now) {
            return { usable: true, allowed: false, reason: 'expired', allowedUntil: record.allowedUntil };
        }
    }

    return {
        usable: true,
        allowed: record.ok !== false,
        reason: 'cached',
        allowedUntil: record.allowedUntil,
        ageDays,
    };
}

module.exports = {
    OFFLINE_GRACE_DAYS,
    statePath,
    read,
    write,
    clear,
    evaluate,
};
