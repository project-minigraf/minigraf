# Durability, Recovery and Operations

This guide is for teams that run Minigraf where losing or corrupting data matters. It covers what survives a crash, what happens when a database is opened after one, where a database can live, how to back it up and restore it, how to check and repair a file, how to upgrade, and what to do about each storage error.

It describes v3.0.0 (file format v8). For v2.x, see the [known issues](https://github.com/project-minigraf/minigraf/issues/421): v2.x checkpoints are not crash-atomic (#374), so some of what follows does not hold there.

## Files and commits

A file-backed database is two files in one directory:

| File | Holds | Lifetime |
|---|---|---|
| `app.graph` | Every checkpointed fact: four indexes, a dictionary, long strings, a free list, and two meta pages | Permanent |
| `app.graph.wal` | Every transaction committed since the last checkpoint, one CRC32-protected entry each | Created by the first write after a checkpoint, deleted by the next checkpoint |

A transaction is **committed** when `execute("(transact ...)")`, `execute("(retract ...)")` or `WriteTransaction::commit()` returns `Ok`. It is then in the WAL and in memory. Each call is one WAL entry, so a transaction is never applied in part: after a crash it is there in full or not at all. A `WriteTransaction` that is rolled back or dropped without `commit()` writes nothing.

A **checkpoint** copies the committed transactions from memory into `app.graph` and deletes the WAL. It runs:

- after every `wal_checkpoint_threshold` WAL entries (default 1,000), inside the write that reaches the threshold;
- when `checkpoint()` is called;
- when the last clone of the handle is dropped (a clean close).

`wal_checkpoint_threshold(usize::MAX)` turns off the first and the last: only `checkpoint()` writes `app.graph`, and the WAL holds everything else.

A checkpoint is copy-on-write. It writes new pages only where the current meta page does not point, syncs them, then writes the other meta page and syncs again. That meta page is the only commit point. A crash at any moment leaves the previous checkpoint intact, and the WAL still holds what the interrupted checkpoint was writing. Every page carries a CRC32, its own page id and the checkpoint generation that wrote it, checked on every read.

Rules (`(rule ...)`) and registered aggregates and predicates live in memory, not in the file. Register them again after every open (rule persistence is #241).

## What survives a crash

| Event | `SyncMode::Full` (default) | `SyncMode::Normal` |
|---|---|---|
| Process crash, `kill -9`, panic with `panic = "abort"`, `std::process::exit` | Every committed transaction | Every committed transaction |
| Operating-system crash or power loss | Every committed transaction | Every transaction up to the last checkpoint, plus whichever later WAL entries reached the disk, in order |
| `Minigraf::in_memory()` | Nothing | Nothing |

A crash during a checkpoint is no different: the file keeps the previous checkpoint, and the WAL is replayed.

**`SyncMode::Full`** calls `fdatasync` on the WAL after every entry before the write returns. When it creates the WAL, it syncs the header and the parent directory, so the new file's name survives a power loss too (#389). A committed transaction survives a power loss.

**`SyncMode::Normal`** writes the WAL entry with no sync. The entry is in the operating system's page cache, which survives the process but not the machine. It reaches the disk at the next checkpoint, which syncs `app.graph` in both modes. After a power loss, replay stops at the first damaged or missing entry, so what comes back is a prefix of the committed transactions, never a transaction in part and never a later one without an earlier one. Use it for bulk loads that can be re-run from a known point, and call `checkpoint()` at the points you would re-run from. With `SyncMode::Normal` and `wal_checkpoint_threshold(usize::MAX)`, nothing is synced until you call `checkpoint()`.

Batching is the other lever: `begin_write()`, several `execute()` calls and one `commit()` write one WAL entry with one sync, in either mode.

**What durability assumes.** A sync is only as good as the storage under it. Minigraf assumes that when `fdatasync`/`fsync` (`FlushFileBuffers` on Windows) returns, the data is on stable storage. Disks or controllers with a volatile write cache that ignore flushes, virtual machines with an "unsafe" disk cache mode, and some network or FUSE filesystems break that assumption, and a power loss there can lose committed transactions or tear a page. A torn or rotted page is detected by its checksum on the next read (`STG-029`); it is not silently read.

**Browser (`BrowserDb`).** There is no WAL. Each `transact` and `retract` is a checkpoint whose pages, meta page and freed page keys are written in one IndexedDB `readwrite` transaction, and the returned promise resolves after that transaction completes. Closing the tab before then loses that write, whole. How durable a completed IndexedDB transaction is on a power loss is up to the browser.

**WASI.** The `wasm32-wasip1` build has no file backend: its databases are in memory and end with the process.

## Opening after a crash

Opening a file-backed database:

1. Takes the file lock (see [Where a database can live](#where-a-database-can-live)). A lock held by a crashed process is released by the kernel when it dies, so there is never a stale lock to clean up.
2. Reads both meta pages and opens at the valid one with the higher generation. If only one is valid, the WAL's recorded base generation and the pages in the file are checked to make sure the damaged one did not hold a newer completed checkpoint; if it did, the open fails with `STG-033` instead of silently going back. If the WAL was created at a later generation than the newest meta page, it belongs to a later state of the file and the open fails with `STG-047` (#535).
3. Checks the meta page against the file: unknown feature bits are `STG-034`, a file shorter than the meta page says is `STG-044`.
4. Replays the WAL into memory, skipping entries at or below the file's checkpointed transaction count, and stopping at the first truncated or damaged entry.
5. For a read-write open, cuts the WAL back to the end of its last valid entry and syncs it, so the next commit lands where replay will find it (#513).

Opening never rebuilds an index and never scans the file. A format v7 file (from v2.x) is upgraded the first time it is opened read-write; see [Upgrading](#upgrading).

| Step | Cost | Measured |
|---|---|---|
| Open a checkpointed file | Constant: two meta pages are read, the rest is paged in on demand | 0.12–0.14 ms and 3 MiB of RSS at 100K, 1M and 10M facts ([BENCHMARKS.md](BENCHMARKS.md#performance-envelope-v300-394)) |
| WAL replay | Linear in the WAL's facts; they are held in memory until the next checkpoint | Bounded by `wal_checkpoint_threshold` (1,000 transactions by default) unless that is raised |
| First open of a v7 file | Linear in the file: every fact is read, re-indexed and written once | One time only |

After a crash, the first checkpoint writes the replayed transactions to `app.graph` and deletes the WAL. That happens at the next automatic checkpoint, at `checkpoint()`, or at a clean close.

A read-only open (`OpenOptions::read_only(true)`) applies the WAL in memory but never writes or deletes it, so a reader sees what a crashed writer committed without changing anything on disk.

## Failures while running

When a write or sync fails (a full disk, an I/O error), the call that hit it returns the error, with the operating system's message in it. The handle then protects the file:

- **A WAL write fails.** The transaction is not applied in memory. Whether it is durable is unknown: if its entry reached the disk whole, it comes back on the next open. Every later write through the handle fails with `WAL-007` until the database is reopened or a `checkpoint()` succeeds. Queries keep working.
- **A checkpoint fails** (automatic, explicit, at close, or in `rebuild_indexes()`). It is never retried on that handle, because after a failed `fsync` the kernel may have dropped the written pages while marking them clean. Every later write, checkpoint and index rebuild fails with `STG-046`; queries keep working; the close-time checkpoint is skipped. Nothing committed is lost: the WAL still holds every committed transaction, and `app.graph` holds either the previous checkpoint or the failed one complete.

An automatic checkpoint runs inside the write that reaches the threshold. If it fails, that `transact` or `commit()` returns the error **although its transaction is already committed**: it is in the WAL and in memory. Before you retry a write that returned an I/O error, reopen and check whether it is there.

To recover from either: fix the cause (free disk space, check the device and the kernel log), drop every clone of the handle, and open the database again.

## Where a database can live

| Setup | Status |
|---|---|
| Linux on ext4 or xfs, macOS on APFS, Windows on NTFS, local disk | Tier 1: tested on every change |
| NFSv4 with working locks | Supported with caveats: network filesystems add latency and failure modes local disks do not have |
| NFSv3 without `lockd`, FUSE filesystems that cannot lock | `open()` fails with `STG-027`. `OpenOptions::allow_unlocked(true)` opens anyway; use it only when nothing else can open the file |
| NFSv3 mounted with `-o nolock` | Unsupported for more than one host: the client grants locks locally, so two hosts can both "hold" the lock and corrupt the file. Minigraf cannot detect this |
| Folders synced by Dropbox, iCloud Drive, OneDrive and similar | Avoid: they copy `app.graph` and `app.graph.wal` independently and at any moment |

**Locking.** The lock is a kernel lock on `app.graph` itself (`flock` on Unix, `LockFileEx` on Windows); there is no lock file. A read-write open is exclusive: a second one fails with `STG-025` in the same process and `STG-026` in another (after retrying for about 375 ms). Read-only opens share the file with each other but not with a read-write open. Inside one process, open the file once and clone the handle: clones are cheap and share one database.

**Containers.** Kernel locks work across containers that share a volume on one Linux host, and a crashed container's lock is released when its process dies, whatever its PID namespace (#317). For a rolling update, the new container's `open()` fails with `STG-026` until the old one exits: either stop the old one first (a "recreate" strategy) or retry the open. Two hosts writing one network volume follow the NFS rows above.

**Windows.** The lock is mandatory: while a database is open, no other handle can read `app.graph`, including backup tools, antivirus scanners and a second handle in the same process (os error 33). Close the database before copying it.

## Backups

The WAL is part of the database. A copy of `app.graph` without `app.graph.wal` loses every transaction since the last checkpoint.

**Offline copy (simplest).** Close every handle in every process, then copy the directory's `app.graph`, and `app.graph.wal` if there is one. After a clean close the WAL is normally gone, unless the handle was opened with `wal_checkpoint_threshold(usize::MAX)` or a checkpoint failed.

**Filesystem or volume snapshot.** An atomic snapshot of the volume holding both files (ZFS, btrfs, LVM, a cloud disk snapshot) is crash-consistent: restoring it is the same as recovering from a power loss at that instant, and the [table above](#what-survives-a-crash) applies. A copy made file by file is not a snapshot.

**Online copy from the running process.** A process that has the database open can copy it to a new file without stopping writers, with `fact_log` and `LogWriter`:

```rust
use minigraf::{FactFilter, LogWriter, OpenOptions};

// `db` is the application's open handle; the target must not exist.
let log = db.fact_log(&FactFilter::new())?;
let tx_count = db.current_tx_count();
let mut out = LogWriter::create("backup/app.graph", OpenOptions::new())?;
for record in log {
    out.append(&record?)?;
}
out.advance_tx_count(tx_count)?;
out.finish()?;
```

- The log reads the database as it was when it opened, uncheckpointed transactions included. Writes and checkpoints go on meanwhile. The log pins the committed generation it opened on, so checkpoints do not reuse the pages it reads: on a database taking many writes, the file grows by the pages checkpoints replace until the log is dropped, and later checkpoints reuse them. A query cursor (`Minigraf::query`) pins the same way until it is closed or dropped.
- Read the transaction count after opening the log, so it covers every logged transaction. The copy keeps every fact version with its `tx_count`, `tx_id` and valid-time window, so `:as-of` and `:valid-at` answer the same; internal ids and page layout are new.
- The copy is built at `backup/app.graph.partial` and renamed into place by `finish()`, so an interrupted backup never leaves a partial file under the final name.

A process that does not have the database open can do the same through a read-only open, but only while no read-write handle is open anywhere.

**Do not** copy `app.graph` while a read-write handle is open: a checkpoint can run during the copy, the copy misses the WAL, and on Windows the read fails.

### Restoring

1. Stop every process that uses the database.
2. Move the current `app.graph` and `app.graph.wal` aside rather than deleting them.
3. **Make sure no `app.graph.wal` is left in the directory**, then copy in the backup's `app.graph`, and its `app.graph.wal` if the backup has one. A WAL from a later state of the database, next to an older `app.graph`, would leave a gap in the history; open refuses it with `STG-047` (#535). v2.x cannot detect it and replays it.
4. Open the database and run `verify()` (below).

## Checking and repairing a file

Every page read checks the page's checksum, id and generation, so a torn or rotted page is an error on the read that reaches it (`STG-029`, `STG-030`, `STG-031`). `verify()` finds damage those checks cannot see:

```rust
let report = db.verify()?;
if !report.is_ok() {
    for problem in &report.problems {
        eprintln!("{problem}");
    }
}
```

It walks every committed page (O(file) time, O(pages) memory) and reports index trees that disagree with EAVT (`STG-038`), keys out of order or pages reached twice (`STG-039`), dictionary problems (`STG-036`, `STG-040`), free-list problems (`STG-035`) and page read errors, one structured error each. It holds the write lock, so writes wait while queries go on; a read-only handle can run it too. Uncheckpointed transactions are not checked: they are in memory and in the WAL, whose entries carry their own CRC. Run it after a restore, after a crash on storage you do not fully trust, and on a schedule if the data is critical.

If `verify()` reports problems:

- **Index or free-list damage** (`STG-038`, `STG-039` or a page error inside an index tree, `STG-035`): call `rebuild_indexes()`. It rebuilds all four indexes from one that is intact and agrees with another (or the only intact one), derives the free list again, and commits like a checkpoint, uncheckpointed writes included. It takes O(facts) time and memory, and a crash during it leaves the previous checkpoint. Run `verify()` again afterwards.
- **`rebuild_indexes()` fails with `STG-041`**, or the dictionary or a value page is damaged: no index can serve as the source. The file is not modified. Restore from a backup.

Copy the damaged `app.graph` and `app.graph.wal` somewhere safe before repairing or restoring, so the damage can be examined later.

## Upgrading

### From v2.x (format v7) to v3

v3 upgrades a v7 file to v8 the first time it opens it read-write. The upgrade re-indexes every fact, is crash-safe (the next open finishes or repeats an interrupted upgrade), and is one way: v2.x cannot open a v8 file (`STG-006`). A WAL left by v2.x is replayed after the upgrade.

1. Stop the application so the database is closed cleanly.
2. Back up `app.graph`, and `app.graph.wal` if there is one. v7 fact pages have no checksums, so keep this copy until the upgraded file has passed `verify()` and your own checks.
3. Optional dry run: open the file with `OpenOptions::read_only(true)` under v3. It reads the v7 file into memory without upgrading it (memory grows with the file), so you can run your queries against v3 first.
4. Open it read-write under v3. The first open takes time proportional to the file.
5. Run `verify()`.

Some query results change in v3 for existing data; check them against [Upgrading to v3](https://project-minigraf.github.io/minigraf-docs/latest/upgrading-to-v3/).

**Rolling back to v2.x** means restoring the backup from step 2: transactions committed under v3 are not in it. There is no v8-to-v7 writer. To carry later data back, re-apply it through v2.x `transact`: valid time can be kept with `:valid-from`/`:valid-to`, transaction time cannot.

### Within v3

Patch and minor releases of v3 read every v8 file. A later v3.x release may add an optional on-disk structure and mark it in the meta page's feature bits; an older v3.x then refuses that file with `STG-034` rather than misread it, so going back to an older library after such a release needs a backup from before it. Formats v1–v6 are not readable by v3 (`STG-028`): open them once with v2.x first.

## Errors: retry, reopen, or restore

Every error has a code (`MinigrafError::code()`); the [error reference](https://project-minigraf.github.io/minigraf-docs/latest/error-reference/) describes each one. For the storage and WAL errors:

| What to do | Codes |
|---|---|
| **Retry later.** Nothing is wrong with the file | `STG-026` (another process holds the database: wait for it to exit) |
| **Fix the setup.** The file is fine | `STG-025` (this process already has the file open: reuse that handle), `STG-027` (the filesystem cannot lock), `STG-042` (no file at the path, read-only open), `STG-043` (the log writer's target exists), `WAL-006` (the WAL could not be deleted after a checkpoint, usually directory permissions; nothing is lost) |
| **Reopen.** The handle refuses writes; every committed transaction is in the WAL | `STG-046`, `WAL-007`, an I/O error from a write or checkpoint (fix the cause first), `API-001` (a thread panicked while holding the write lock) |
| **Use another library version.** The file is not modified | `STG-006` (format newer than this library), `STG-034` (feature bits from a newer v3.x), `STG-028` (format older than v7: open with v2.x first), `WAL-002` (WAL from another version: open with that version and `checkpoint()`) |
| **Repair.** Found by `verify()` | `STG-035`, `STG-038`, `STG-039` → `rebuild_indexes()`. `STG-036`, `STG-040` → restore |
| **Restore from backup.** The file is damaged or is not this database; a failed open leaves it unmodified | `STG-029`, `STG-030`, `STG-031` (a damaged page; inside an index, try `rebuild_indexes()` first), `STG-032`, `STG-033`, `STG-041`, `STG-044`, `STG-045`, `STG-002` (not a Minigraf file: check the path) |
| **Restore, or accept losing the WAL** | `WAL-001` (the `.wal` file is not a WAL), `STG-047` (the `.wal` is from a later state than `app.graph`, usually a restore that left the current WAL in place: move the WAL aside, or restore the `app.graph` it belongs to). Copy both files first. Deleting the `.wal` opens the database without the transactions since the last checkpoint |

A write that returns an I/O error may still have committed (see [Failures while running](#failures-while-running)); check before writing it again.

## Known limitations

- **A damaged WAL entry ends replay.** Replay stops at the first entry whose CRC fails, and a read-write open then cuts the WAL there, so a bit flip in the middle of the WAL loses that entry and every later one without an error. With `SyncMode::Full` this needs damage after the sync; a torn entry from a crash is always the last one.
- **One writing process at a time.** A database is single-writer; a second process gets `STG-026`. Share one handle inside a process, or put one process in front of the database.
- **No online backup from another process.** A read-only open needs every read-write handle closed. Copy from inside the writing process (see [Backups](#backups)), or use a volume snapshot.
- **Rules and registered functions are not stored** in the file (#241). Register them after every open.
- **Uncheckpointed transactions are held in memory.** A very large `wal_checkpoint_threshold` grows memory and WAL replay time with the pending transactions. A fact log or cursor left open keeps the transactions that were uncheckpointed when it opened in memory, and the pages later checkpoints replace in the file, until it is dropped; a crash or reopen frees those pages.

Open bugs that affect stored data or recovery carry the [`data-integrity`](https://github.com/project-minigraf/minigraf/issues?q=is%3Aopen+label%3Adata-integrity) and [`durability`](https://github.com/project-minigraf/minigraf/issues?q=is%3Aopen+label%3Adurability) labels; a fix merged for an unreleased version stays open until it ships. Bugs in the current published release, with workarounds and fix versions, are listed in [#421](https://github.com/project-minigraf/minigraf/issues/421).
