# Fourth-session handoff

## Sources already combined

- Verified desktop branch: hoplite/barke-6b3e18a3 at 06c7a1bb65f84a77684a8c4e1452bcf2ffa28a22.
- WIP safety branch: hoplite/barke-6b3e18a3-wip at 43dcc1c5ac3969636d3c543b4a46d3a275e49371.
- Merged without conflicts in this handoff. Both original histories are retained; do not replay them.
- The merge adds workbook builder, migration 0032, builder tests and presets.json to the verified desktop branch.

## Verification and limits

The prior session records 108 core library tests and 235 app tests passing on an isolated clean desktop checkpoint. These tests were not rerun during the transfer. Live suites returned early without configuration and establish no backend integration coverage. A Windows cross-build was started but successful completion was not recorded. No completed UI screenshots or real Excel macro verification were recorded.

The merged source remains WIP. It must not be deployed before full verification. New backend Python files were parsed for syntax in the handoff; this is not a runtime or behavioral test.

## Remaining priorities

1. Verify the combined Rust source and existing tests, complete visual checks for filters rather than rewriting the already integrated work.
2. Audit/finish the preserved Excel builder and route integration; add missing test_excel_workbook_api.py, run builder/API/service/VBA tests, migration round trips and isolated-local activation/snapshot/revocation tests.
3. Address P1.7 Operation input/parity/export gaps from IMPLEMENTATION_PLAN.md.
4. Finish P3 acceptance checks and P4 View implementation.
5. Complete Windows build, real interface inspection, actual Excel execution if available, then document a deployment proposal. Do not modify production automatically.

The implementation plan contains some stale recovery statements; update it based on actual source and fresh checks. Older docs/handoff/RECOVERY_STATUS.md is historical.

## Access and preservation

A shared SSH key has not been set up in this handoff. Previous session private keys do not transfer. Follow CONTINUE_PROMPT.md for secure provisioning and existing read-only restrictions. No private key is embedded in the prompt or repository.

Commit and PUSH after every substantial milestone, include intended new files, verify remote SHA, and push clearly labelled WIP before credits expire. Run work sequentially by default to conserve credits.
