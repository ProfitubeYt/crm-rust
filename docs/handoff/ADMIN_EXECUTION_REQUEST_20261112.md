# Request for administrator-assisted execution — dictionary import and historical fills

Prepared 2026-11-12 on branch `work/dictionary-and-details`. **Nothing below has been run on
production.** The session that prepared it holds only the SELECT-only role `hoplite_ro`.

This asks for one thing: that an administrator, with the owner's approval and a fresh verified
backup, runs a reviewed change manifest with the journalled tool described here. It needs no
deployment, no migration and no restart, and it does not widen the read-only account.

## 1. What would be written

Measured read-only on live production on 2026-11-12 (1 938 receipts, schema `20261112_0033`):

| Part | Rows | What |
|---|---|---|
| `dictionary` | 17 new active lines in `operator_reference` | the verified lines of `backend/data/operator_dictionary/research_20261112.json` |
| `applications` | 656 receipts | empty «Приложение» filled, with `application_src` (`dict` 270, `rule` 386) and `application_confidence` |
| `details` | 59 receipts | empty «Подробности» filled from confirmed patterns, `annotation_details_src = rule`, one journal event each |

Not written by this plan, on purpose: the 46 «Подробности» suggestions; the mappings under
`needs_owner_review`; any value a person set; any value of unknown origin (5 stored values that
differ from the evidence are only reported); receipts inside a locked period (none is active today).

Content digest of the reviewed plan: `720660a287af0bc93266c8105b137763b582b3bc1425d6b10065120d5f34df6b`.
It covers the three lists above and nothing else. The same data gives the same digest whoever
builds the manifest; new receipts that add a fill change it, and then the printed counts say by
how much.

## 2. Why a manifest and not the existing `--apply` scripts

`scripts.backfill_applications --apply` and `scripts.enrich_details --apply` of the live 2.0.2
backend would write the same notes, and the 457 application fills that need no new dictionary line.
They keep no per-receipt before-values outside the database, have no batch id and cannot take one
run back. The manifest route gives exactly what the task requires: the exact receipt ids reviewed
before the run, an optimistic check per receipt at the moment of writing, a journal with before and
after values, and an undo of that batch alone.

## 3. Files (all in the branch; none of them is deployed by running them)

| File | Role | Needs |
|---|---|---|
| `backend/scripts/build_change_manifest.py` | builds the manifest; **reads only** | the branch's `backend/` and a database role that can SELECT |
| `backend/scripts/apply_change_manifest.py` | `verify` / `apply` / `rollback`; one file, imports nothing of the application | Python, SQLAlchemy, the driver; the application's role for `apply` and `rollback` |
| `backend/scripts/operator_dictionary.py` | `plan` and `review`; **reads only** | as the first |

The manifest and the journal name receipts and notes. They are written with mode 600 and belong in
a root-only directory such as `/opt/parcer-backups/<batch>/`, never in git, a ticket or a chat.

## 4. Procedure

Names of the compose project, network and image are the server's; the account that prepared this
cannot see them. `<…>` marks what the administrator fills in.

```bash
BATCH=dict-details-$(date -u +%Y%m%d)
B=/opt/parcer-backups/$BATCH
install -d -m 700 "$B"

# 1. Fresh backup, then prove it can be read (a restore into a scratch database is better still).
docker exec <postgres container> pg_dump -U <user> -Fc <database> > "$B/pre.dump"
docker exec -i <postgres container> pg_restore --list < "$B/pre.dump" | wc -l
sha256sum "$B/pre.dump" > "$B/pre.dump.sha256"

# 2. The reviewed sources, beside the backup and not over /opt/receipt-parser.
git clone --depth 1 --branch work/dictionary-and-details https://github.com/borianahameldoluno24-debug/fix.git "$B/src"
git -C "$B/src" rev-parse HEAD            # must be the SHA named in the final report

# 3. Build the manifest: a one-off container of the live image, the branch's backend read-only.
run() { docker run --rm --network <compose network> --env-file /opt/receipt-parser/.env \
          -v "$B/src/backend:/work:ro" -v "$B:/out" -w /work <live backend image> "$@"; }
run python -m scripts.build_change_manifest --out /out/manifest.json --batch-id "$BATCH"
#    Compare the printed counts and digest with section 1.

# 4. Check, apply, check again.
run python scripts/apply_change_manifest.py verify --manifest /out/manifest.json
run python scripts/apply_change_manifest.py apply  --manifest /out/manifest.json \
      --journal /out/journal.json --i-have-a-verified-backup
run python scripts/apply_change_manifest.py verify --manifest /out/manifest.json   # every part: done

# 5. After two minutes (the workers' dictionary cache), the live code must agree:
docker exec <backend container> python -m scripts.backfill_applications   # filled from evidence: 0
docker exec <backend container> python -m scripts.enrich_details          # would be filled: 0
```

`apply` may be run in parts: `--only dictionary`, then `--only applications`, then `--only details`,
each with its own journal file. A journal file is never written over.

### 4.1 Turning the note rules on for new receipts (one more decision)

The fills above do not create the rules that give **new** receipts their notes. That is one run of
the script the live backend already has:

```bash
docker exec <backend container> python -m scripts.enrich_details --apply
# expected: rules created 11, updated 0, disabled 14; receipts changed 0
```

It relearns the note rules from the confirmed receipts and writes them to `annotation_rules`; the
14 operator-only rules of the earlier release are switched off and kept. Because the 59 notes are
already in place it changes no receipt. From then on the existing five-minute background pass
(`ANNOTATION_AUTOFILL_ENABLED`) fills the note of a new receipt that matches a confirmed pattern.
`--revalidate` would also take back 6 notes the earlier operator-only rules wrote; that is not part
of this plan and is the owner's call.

### 4.2 What to look at afterwards

- `operator_reference`: 87 active lines (70 + 17), 95 inactive untouched.
- «Приложение» empty on about 1 004 receipts instead of 1 660; «Подробности» empty on about 679.
- A receipt that arrives with one of the new operators carries its application at once.
- Desktop and workbook show the values after their next synchronisation (`updated_at` is bumped).

## 5. Taking the batch back

```bash
run python scripts/apply_change_manifest.py rollback --journal /out/journal.json
sleep 180
run python scripts/apply_change_manifest.py rollback --journal /out/journal.json   # expect: undone 0
```

- Only the records of that journal are touched, and only where the value the batch wrote is still
  there. A receipt a person edited afterwards, a dictionary line somebody changed and every receipt
  that arrived later stay as they are and are listed as kept.
- A note that is taken back would be written again within minutes if its rule stayed on, so the
  computed rules behind those notes are switched off (never a rule a person pinned). The second run
  catches a note a worker wrote from rules it still had cached. The next relearning switches the
  rules on again.
- Receipts parsed between the import and the rollback keep the application the new lines gave them:
  they are newer receipts, not the batch. `python -m scripts.backfill_applications --revalidate`
  (dry run first) lists them and, with `--apply`, clears or corrects them.
- It never restores `pre.dump`. A whole-database restore undoes everybody's later work and needs its
  own, separately coordinated authorisation.

## 6. Evidence that the procedure works

Rehearsed on 2026-11-12 on a private PostgreSQL 15.19 copy with the repository's schema and the
production values of the affected columns (no receipt text):

- the manifest built there has the same digest as the one built from live production;
- `verify` → ready 17 / 656 / 59; `apply` refused without the backup flag, then wrote 17 / 656 / 59;
- afterwards the live scripts found nothing to fill and nothing stale; `enrich_details --apply`
  created 11 rules, switched 14 off and changed no receipt;
- a second `apply` wrote nothing;
- `rollback` took back 17 / 656 / 59 and the affected columns of all 1 934 receipts and the
  dictionary were byte-for-byte what they were before.

`verify` was also run against live production through the read-only role: ready 17 / 656 / 59.

## 7. Separate proposal: deploying the branch

Not requested here and not authorised by the task. When a release is planned, the branch adds:

- backend: the reference routes drop the dictionary cache at once after an edit; `GET
  /api/reference/review` lists operators that wait for a decision; opt-in relearning of the note
  rules (`ANNOTATION_DETAILS_RULES_REFRESH_HOURS`, off by default); no schema change;
- desktop: the table and the view panel without «Описание», with saved views migrated. It needs a
  new installer and a version number, as 2.0.2 did.
