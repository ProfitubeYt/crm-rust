# Shared read-only SSH access

Installed and tested on 2026-11-11. The private key is NOT in this repository.

- Host: `64.188.106.221`, port `22`, user `hoplite-ro`.
- Client key fingerprint: `SHA256:WEfc++gQxOhnrZCuD+xOfnaBGm0T6djPq8FmHZyDpYE`.
- Verified server ED25519 fingerprint: `SHA256:v6EsGV568CnEfD2uUH7tu3pZuXpgJbwjbR9HiXU22SM`.
- Owner's private file: `C:/Users/heper/.ssh/hoplite-shared-ro-ed25519`, Windows ACL restricted to the owner and SYSTEM. This is a local path, not an automatic sandbox mount.
- Public file: same path with `.pub` appended.
- Expiry: key and DB role both stop authenticating at 2026-11-25 00:00 UTC unless renewed by the administrator.

## Provisioning another sandbox

Use an actually supported protected secret or file mounting mechanism to supply the private key OUTSIDE the tracked worktree. Do not paste the key into chat, a prompt, a task attachment that becomes repository content, or git. Set `HOPLITE_SSH_KEY_PATH` to the supplied file's actual path; the environment variable contains a path, not the private key itself. Restrict the private file to mode 600 in Linux. If Hoplite does not offer a secure persistent mechanism, server installation cannot by itself make the key available in fresh sandboxes; solve provisioning explicitly rather than generating keys repeatedly or putting a secret into the prompt.

Verify the client-key fingerprint and pin the server key before connecting. Do not blindly accept an unverified scanned host key. With proper known_hosts provisioning, use `-o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -i "$HOPLITE_SSH_KEY_PATH"`.

The only allowed exec command is `pgpass`, to read the private DB credential. Retrieve it directly into protected process memory or a mode-600 credential file; never print or commit it. Forward only to `127.0.0.1:9990`. PostgreSQL user is `hoplite_ro`, with SELECT-only grants, statement timeout 30s and connection limit 4. No shell, sudo, Docker, reverse forwarding, or private authentication-data access is granted.

Tested at installation: authentication succeeds, pgpass returns the expected credential layout without displaying its contents, the DB tunnel opens, and an ordinary shell command is denied. Existing per-session authorized keys were preserved.

## Revocation

The administrator can revoke this shared key for future connections by backing up `/home/hoplite-ro/.ssh/authorized_keys` and removing only the line whose public-key fingerprint matches the client fingerprint above. Existing established sessions/tunnels need to be disconnected separately for immediate revocation. Dropping/disabling `hoplite_ro` revokes database login for every client using that role and should be coordinated if other sessions still need it. Do not delete unrelated keys without instruction.
