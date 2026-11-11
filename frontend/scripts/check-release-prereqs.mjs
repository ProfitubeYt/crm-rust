#!/usr/bin/env node
/**
 * Fail the build when a required-but-gitignored file is missing.
 *
 * `client-access.json` is copied into the package by build.extraResources.
 * It is gitignored, so a fresh clone does not have it — and electron-builder
 * only prints "file source doesn't exist" and carries on. That warning scrolls
 * past in a hundred lines of build output, and the result is an installer that
 * starts up to "access denied" for every user who auto-updates to it.
 *
 * That happened once. This turns the warning into a stop.
 */
import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const frontendDir = resolve(here, '..');

let failed = false;

function fail(message, hint) {
    failed = true;
    console.error(`\n  ✗ ${message}`);
    if (hint) console.error(`    ${hint}`);
}

// --- client-access.json ----------------------------------------------------
const configPath = resolve(frontendDir, 'client-access.json');
if (!existsSync(configPath)) {
    fail(
        'frontend/client-access.json is missing.',
        'It is gitignored but required: build.extraResources copies it into\n'
        + '    resources/security/client-access.json, and the app refuses to start\n'
        + '    without it. Copy it from the server (/opt/receipt-parser/frontend/)\n'
        + '    or from security/client-access.example.json and fill in real values.',
    );
} else {
    try {
        const config = JSON.parse(readFileSync(configPath, 'utf8'));
        const token = config.system_access_token;
        if (!token || token === 'change_me_system_token') {
            fail(
                'frontend/client-access.json still holds the placeholder token.',
                'Shipping it would lock every user out exactly as a missing file does.',
            );
        }
        if (!config.api_base_url) {
            fail('frontend/client-access.json has no api_base_url.');
        }
    } catch (error) {
        fail(`frontend/client-access.json is not valid JSON: ${error.message}`);
    }
}

if (failed) {
    console.error('\n  Release build aborted.\n');
    process.exit(1);
}

console.log('  ✓ release prerequisites present');
