# Windows COLMAP install rename regression

## Failure observed

- Main commit: `2db1f6fa38c0eabd89a9950e79edc836f5319de0`.
- [Windows Actions run 40276323664](https://github.com/ooolabdev/ooosplat/actions/runs/40276323664), job `121419734430`, failed in **Install and verify pinned engines**.
- `setup-colmap-runtime.mjs` reported `EPERM` when renaming the verified staging directory into `engines/colmap`. Frontend checks had already passed; Rust checks and installer packaging were not reached.
- The log identifies the failed filesystem operation, not the process holding a handle. Antivirus interference is possible but not established by that log.

## Change

- Directory publication, backup and rollback use bounded retries on Windows for `EPERM`, `EACCES` and `EBUSY`: at most nine attempts, with 9.1 seconds of total backoff per rename.
- Other errors and other platforms retain immediate failure behavior. Permanent access errors still fail, preserving the original filesystem error.
- SHA-256, release identity, inventory and native CLI checks remain mandatory. There is no copy fallback or change to the pinned runtime.
- An unsuccessful rollback retains the previous runtime in the transaction directory, reports its recovery path and preserves both error causes. Installer cleanup must not delete that backup.

## Local validation — 2026-10-26

Windows x64, Node.js `22.17.0`; the failed hosted job used Node.js `24.22.1`.

- `npm run test:engine-scripts`: **75 passed, 8 platform-specific skips**. Added coverage for transient and persistent locks, fail-fast errors, publication, verification failure and rollback.
- The native Windows test holds a staging file open with a PowerShell `FileStream` that denies deletion. A direct rename fails; publication succeeds after the handle is released. This is a real filesystem lock test, not just an injected error.
- Copied the installed hash-locked COLMAP runtime to an isolated `.cache/ci-colmap-rename/native-install-*` transaction, verified it, published it with the production commit function and verified it again. Both staged and published runtimes passed native CLI probes. The original runtime also passed full verification afterward; no existing runtime was replaced.
- `npm test`: **586 frontend tests and 6 diagnostic tests passed**.
- `npm run build`: passed; existing browser externalization, WebP URL and chunk-size warnings remain.
- COLMAP lock, bundle resource and license checks passed.

No Rust source, workflow policy, application version, model or project files changed. No installer was generated. This local validation is not a successful rerun of hosted Actions: Node 24, full Windows packaging, and native Linux/macOS installation still require CI validation after the change is pushed.
