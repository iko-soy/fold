use assert_cmd::Command;
use predicates::prelude::*;

fn notes(dir: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("fold").unwrap();
    c.arg("--vault").arg(dir);
    c
}

#[test]
fn empty_vault_gets_fresh_root() {
    let dir = tempfile::tempdir().unwrap();
    notes(dir.path())
        .arg("check")
        .assert()
        .success();
    let root = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(root.starts_with("# Inbox"), "{}", root);
}

#[test]
fn capture_appends_under_today() {
    let dir = tempfile::tempdir().unwrap();
    notes(dir.path())
        .args(["capture", "hello from cli"])
        .assert()
        .success();
    let root = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(root.contains("- hello from cli"), "{}", root);
    // dated day section
    assert!(root.contains("## 20"), "{}", root);
}

#[test]
fn capture_task_and_to() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# Projects\n").unwrap();
    notes(dir.path())
        .args(["capture", "buy milk", "--task", "--to", "Projects"])
        .assert()
        .success();
    let root = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(root.contains("- [ ] buy milk"), "{}", root);
    let proj = root.find("# Projects").unwrap();
    let milk = root.find("buy milk").unwrap();
    assert!(milk > proj);
}

#[test]
fn capture_from_stdin() {
    let dir = tempfile::tempdir().unwrap();
    notes(dir.path())
        .args(["capture"])
        .write_stdin("from stdin\n")
        .assert()
        .success();
    let root = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(root.contains("- from stdin"), "{}", root);
}

#[test]
fn check_reports_and_fix_canonicalizes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n\n* [X] old style\n").unwrap();
    notes(dir.path())
        .args(["check"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("non-canonical"));
    notes(dir.path())
        .args(["check", "--fix"])
        .assert()
        .success();
    let root = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(root.contains("- [x] old style"), "{}", root);
}

#[test]
fn merge_processes_conflict_files() {
    let dir = tempfile::tempdir().unwrap();
    // the merged conflict file goes to a trash of the test's own (§11.5)
    let state = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n\n- ours\n").unwrap();
    std::fs::write(
        dir.path().join("root.sync-conflict-20260912-100000-phone.md"),
        "# A\n\n- theirs\n",
    )
    .unwrap();
    notes(dir.path())
        .env("XDG_STATE_HOME", state.path())
        .args(["merge"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sync-conflict"));
    let root = std::fs::read_to_string(dir.path().join("root.md")).unwrap();
    assert!(root.contains("- ours") && root.contains("- theirs"), "{}", root);
}

#[test]
fn trash_list_and_restore() {
    let dir = tempfile::tempdir().unwrap();
    // a trash of the test's own (§11.5), not the machine's, where an entry
    // left by an earlier run would make "doomed" match twice
    let state = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# A\n").unwrap();
    // a deleted subtree, as a delete leaves it in the trash
    let trash = state.path().join("fold").join("trash");
    std::fs::create_dir_all(&trash).unwrap();
    std::fs::write(trash.join("20260926-101010-doomed.md"), "- doomed\n").unwrap();
    notes(dir.path())
        .env("XDG_STATE_HOME", state.path())
        .args(["trash", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("doomed"));
    // restore by a unique substring of the trash file name
    notes(dir.path())
        .env("XDG_STATE_HOME", state.path())
        .args(["trash", "restore", "doomed"])
        .assert()
        .success();
    assert!(dir.path().read_dir().unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("doomed")));
}

#[test]
fn help_prints_usage() {
    let dir = tempfile::tempdir().unwrap();
    notes(dir.path())
        .args(["help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("command palette"))
        .stdout(predicate::str::contains("make block"));
}

#[test]
fn missing_vault_is_an_error_not_a_new_directory() {
    let dir = tempfile::tempdir().unwrap();
    let typo = dir.path().join("typo");
    notes(&typo).arg("check").assert().failure();
    assert!(!typo.exists());
}

#[test]
fn trash_restore_never_overwrites_root_md() {
    let dir = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.md"), "# Keep\n").unwrap();
    let trash = state.path().join("fold").join("trash");
    std::fs::create_dir_all(&trash).unwrap();
    std::fs::write(trash.join("20260926-101010-root.md"), "# Root\n").unwrap();
    notes(dir.path())
        .env("XDG_STATE_HOME", state.path())
        .args(["trash", "restore", "root"])
        .assert()
        .success();
    assert_eq!(std::fs::read_to_string(dir.path().join("root.md")).unwrap(), "# Keep\n");
    assert!(dir.path().join("root-restored-2.md").exists());
}

/// The trash lives outside the vault (§11.5), often on another filesystem
/// (a Syncthing folder on an external disk vs `$XDG_STATE_HOME`). Merging
/// must still move the conflict file to trash (copy + remove across
/// filesystems), or every rerun merges the same conflict again and adds
/// another duplicate conflict block.
#[test]
#[cfg(target_os = "linux")]
fn merge_trashes_conflict_file_across_filesystems() {
    use std::os::unix::fs::MetadataExt;
    let dir = tempfile::tempdir().unwrap();
    let Ok(state) = tempfile::tempdir_in("/dev/shm") else {
        return;
    };
    let dev = |p: &std::path::Path| std::fs::metadata(p).unwrap().dev();
    if dev(dir.path()) == dev(state.path()) {
        return; // not a cross-filesystem setup on this machine
    }
    std::fs::write(dir.path().join("root.md"), "# A\n\n- [ ] t\n").unwrap();
    let c = "root.sync-conflict-20260912-100000-phone.md";
    std::fs::write(dir.path().join(c), "# A\n\n- [x] t\n").unwrap();
    notes(dir.path())
        .env("XDG_STATE_HOME", state.path())
        .arg("merge")
        .assert()
        .success();
    assert!(!dir.path().join(c).exists(), "conflict file left in the vault");
    let trash = state.path().join("fold").join("trash");
    assert_eq!(std::fs::read_dir(&trash).unwrap().count(), 1);
    // idempotent: a second run finds nothing to merge and adds no blocks
    let md_files = |d: &std::path::Path| {
        std::fs::read_dir(d)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "md"))
            .count()
    };
    let before = md_files(dir.path());
    notes(dir.path())
        .env("XDG_STATE_HOME", state.path())
        .arg("merge")
        .assert()
        .success()
        .stdout(predicate::str::contains("no sync-conflict files"));
    assert_eq!(md_files(dir.path()), before);
}

// ------------------------------------------------------------ the TUI in a terminal

/// `fold` in a terminal of its own, the one util-linux `script` gives it:
/// what it writes there, keys typed into it, and its pid.
#[cfg(target_os = "linux")]
struct Tty {
    script: std::process::Child,
    keys: std::process::ChildStdin,
    out: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    /// reads what script passes on; done once script closes its output
    reader: std::thread::JoinHandle<()>,
    /// while set, the reader reads nothing (`stall`)
    stalled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pid: u32,
}

/// Poll `f` for up to 10 s until it gives something.
#[cfg(target_os = "linux")]
fn until<T>(mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(t) = f() {
            return Some(t);
        }
        if std::time::Instant::now() > end {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(target_os = "linux")]
impl Tty {
    /// fold on `vault`, with the normal keymap, in a 120×32 terminal, its
    /// trash under `state`; None where there is no `script` to run it in.
    fn start(vault: &std::path::Path, state: &std::path::Path) -> Option<Tty> {
        use std::io::Read;
        use std::process::Stdio;
        let pidfile = state.join("fold.pid");
        let cmd = format!(
            "stty cols 120 rows 32; echo $$ > '{}'; exec '{}' --keys normal --vault '{}'",
            pidfile.display(),
            env!("CARGO_BIN_EXE_fold"),
            vault.display()
        );
        let script = std::process::Command::new("script")
            .args(["-qfec", &cmd, "/dev/null"])
            .env("SHELL", "/bin/sh")
            .env("TERM", "xterm-256color")
            .env("XDG_STATE_HOME", state)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut script) = script else {
            eprintln!("skipped: no script(1) to give fold a terminal");
            return None;
        };
        let keys = script.stdin.take().unwrap();
        let mut stdout = script.stdout.take().unwrap();
        let out = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = out.clone();
        let stalled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let held = stalled.clone();
        let reader = std::thread::spawn(move || {
            let mut buf = [0; 4096];
            loop {
                // a stalled terminal keeps its end open and reads nothing
                while held.load(std::sync::atomic::Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                let Ok(n @ 1..) = stdout.read(&mut buf) else { return };
                sink.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
        let pid = until(|| std::fs::read_to_string(&pidfile).ok()?.trim().parse().ok());
        let Some(pid) = pid else {
            let _ = script.kill();
            eprintln!("skipped: script(1) ran no shell");
            return None;
        };
        Some(Tty { script, keys, out, reader, stalled, pid })
    }

    fn output(&self) -> String {
        String::from_utf8_lossy(&self.out.lock().unwrap()).to_string()
    }

    /// The last of the output, for a failure's message.
    fn tail(&self) -> String {
        let out = self.output();
        let from = out.char_indices().rev().nth(600).map_or(0, |(i, _)| i);
        format!("{:?}", &out[from..])
    }

    /// Whether fold draws `what` within 10 s.
    fn shows(&self, what: &str) -> bool {
        until(|| self.output().contains(what).then_some(())).is_some()
    }

    fn type_keys(&mut self, bytes: &str) {
        use std::io::Write;
        self.keys.write_all(bytes.as_bytes()).unwrap();
        self.keys.flush().unwrap();
    }

    /// Stop reading what fold writes, as a terminal that has stalled (an
    /// ssh session gone quiet): once the pipes fill, fold's writes wait.
    fn stall(&self) {
        self.stalled.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// The syscall fold's main thread is in, by number; None where
    /// /proc does not say.
    fn syscall(&self) -> Option<String> {
        let s = std::fs::read_to_string(format!("/proc/{}/syscall", self.pid)).ok()?;
        Some(s.split(' ').next()?.trim().to_string())
    }

    fn signal(&self, sig: &str) {
        let pid = self.pid.to_string();
        std::process::Command::new("kill").args(["-s", sig, &pid]).status().unwrap();
    }

    /// Whether fold has exited: reaped, or a zombie no one has reaped yet.
    fn exited(&self) -> bool {
        let s = std::fs::read_to_string(format!("/proc/{}/stat", self.pid)).unwrap_or_default();
        let state = s.rsplit(") ").next().and_then(|r| r.chars().next());
        matches!(state, None | Some('Z'))
    }

    /// Whether fold is gone within 10 s (reaped, or a zombie no one reaps)
    /// and all it wrote has come through: its last bytes can wait in the
    /// terminal after it exits, until script passes them on and closes its
    /// output.
    fn ended(&self) -> bool {
        until(|| (self.exited() && self.reader.is_finished()).then_some(())).is_some()
    }

    /// Open the editor on the first node, and paste ` MYTEXT` at the end of
    /// its title line.
    fn edit(&mut self) {
        assert!(self.shows("Snapshot"), "fold drew nothing:\n{}", self.tail());
        self.type_keys("e");
        assert!(self.shows("EDIT"), "no editor:\n{}", self.tail());
        // End, then a bracketed paste
        self.type_keys("\x1b[F\x1b[200~ MYTEXT\x1b[201~");
        assert!(self.shows("MYTEXT"), "nothing typed:\n{}", self.tail());
    }
}

#[cfg(target_os = "linux")]
impl Drop for Tty {
    fn drop(&mut self) {
        let _ = std::process::Command::new("kill")
            .args(["-9", &self.pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status();
        let _ = self.script.kill();
        let _ = self.script.wait();
        // the reader reads to the end and is done
        self.stalled.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

/// A vault named as a real one is (tempfile's own names start with a dot),
/// and a state directory for its trash.
#[cfg(target_os = "linux")]
fn vault_and_state(root: &str) -> (tempfile::TempDir, tempfile::TempDir) {
    let vault = tempfile::Builder::new().prefix("vault").tempdir().unwrap();
    std::fs::write(vault.path().join("root.md"), root).unwrap();
    (vault, tempfile::tempdir().unwrap())
}

/// Where the output puts the terminal back as it was: out of the
/// alternate screen, the mouse and bracketed paste turned off before.
#[cfg(target_os = "linux")]
fn put_back(out: &str) -> Option<usize> {
    let at = out.rfind("\x1b[?1049l")?;
    let tail = &out[out.rfind("\x1b[?1049h")?..at];
    (tail.contains("\x1b[?1000l") && tail.contains("\x1b[?2004l")).then_some(at)
}

#[test]
#[cfg(target_os = "linux")]
fn a_signal_saves_the_editor_and_puts_the_terminal_back() {
    let (vault, state) = vault_and_state("# Snapshot policy\n\nkeep 24\n");
    let Some(mut tty) = Tty::start(vault.path(), state.path()) else { return };
    tty.edit();
    tty.signal("TERM");
    assert!(tty.ended(), "fold still runs");
    let root = std::fs::read_to_string(vault.path().join("root.md")).unwrap();
    assert_eq!(root, "# Snapshot policy MYTEXT\n\nkeep 24\n");
    assert!(put_back(&tty.output()).is_some(), "terminal left as fold had it:\n{}", tty.tail());
}

#[test]
#[cfg(target_os = "linux")]
fn a_signal_keeps_text_a_save_was_refused_for_in_the_trash_and_says_where() {
    let (vault, state) = vault_and_state("# Snapshot policy\n\nkeep 24\n");
    let Some(mut tty) = Tty::start(vault.path(), state.path()) else { return };
    tty.edit();
    // another program changes the node under the typing: no save can go
    // through
    std::fs::write(vault.path().join("root.md"), "# Snapshot policy\n\nkeep 48\n").unwrap();
    tty.signal("TERM");
    assert!(tty.ended(), "fold still runs");
    let root = std::fs::read_to_string(vault.path().join("root.md")).unwrap();
    assert_eq!(root, "# Snapshot policy\n\nkeep 48\n");
    // said once the terminal is back, so it stays on screen
    let out = tty.output();
    let back = put_back(&out).unwrap_or_else(|| panic!("terminal left as fold had it:\n{}", tty.tail()));
    let said = out[back..].find("unsaved text kept in ").unwrap_or_else(|| panic!("nothing said:\n{}", tty.tail()));
    let path = out[back + said..]["unsaved text kept in ".len()..].lines().next().unwrap().trim_end();
    assert!(path.starts_with(&state.path().join("fold").join("trash").display().to_string()), "{}", path);
    assert!(path.ends_with("-unsaved-snapshot-policy.md"), "{}", path);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "# Snapshot policy MYTEXT\n\nkeep 24\n");
}

#[test]
#[cfg(target_os = "linux")]
fn a_signal_puts_the_terminal_back_though_script_passes_it_on_late() {
    let (vault, state) = vault_and_state("# Snapshot policy\n\nkeep 24\n");
    let Some(mut tty) = Tty::start(vault.path(), state.path()) else { return };
    tty.edit();
    // script(1) falls behind, as on a busy machine: fold's last bytes are
    // still in the terminal when fold has gone
    let script = tty.script.id().to_string();
    let to_script = |sig: &str| {
        std::process::Command::new("kill").args(["-s", sig, &script]).status().unwrap();
    };
    to_script("STOP");
    tty.signal("TERM");
    std::thread::scope(|s| {
        s.spawn(|| {
            until(|| tty.exited().then_some(()));
            std::thread::sleep(std::time::Duration::from_millis(300));
            to_script("CONT");
        });
        assert!(tty.ended(), "fold still runs");
        assert!(put_back(&tty.output()).is_some(), "terminal left as fold had it:\n{}", tty.tail());
    });
}

/// write(2)'s number, as /proc/<pid>/syscall gives it.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const WRITE: &str = "1";
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const WRITE: &str = "64";

#[test]
#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
fn a_second_signal_ends_fold_stuck_on_a_terminal_that_reads_nothing() {
    let root: String = (0..300).map(|i| format!("- item {} of a long outline that fills the screen\n", i)).collect();
    let (vault, state) = vault_and_state(&format!("# Snapshot policy\n\n{}", root));
    let Some(mut tty) = Tty::start(vault.path(), state.path()) else { return };
    assert!(tty.shows("Snapshot"), "fold drew nothing:\n{}", tty.tail());
    if tty.syscall().is_none() {
        eprintln!("skipped: /proc does not say what fold waits on");
        return;
    }
    // the terminal stops reading; fold, redrawing the whole outline from
    // bottom to top and back, fills it and waits in a write
    tty.stall();
    tty.type_keys(&"Ggg".repeat(200));
    let stuck = until(|| (tty.syscall().as_deref() == Some(WRITE)).then_some(()));
    assert!(stuck.is_some(), "fold never waited on the terminal: {:?}", tty.syscall());
    // the first signal asks fold to end as on a quit, which it cannot
    // while it waits; the second ends it all the same
    tty.signal("TERM");
    std::thread::sleep(std::time::Duration::from_millis(300));
    tty.signal("TERM");
    assert!(until(|| tty.exited().then_some(())).is_some(), "fold still runs: {:?}", tty.syscall());
}

#[test]
#[cfg(target_os = "linux")]
fn closing_the_window_saves_the_editor() {
    let (vault, state) = vault_and_state("# Snapshot policy\n\nkeep 24\n");
    let Some(mut tty) = Tty::start(vault.path(), state.path()) else { return };
    tty.edit();
    // the terminal goes away: fold gets SIGHUP, and nowhere to draw
    tty.script.kill().unwrap();
    assert!(tty.ended(), "fold still runs");
    let root = std::fs::read_to_string(vault.path().join("root.md")).unwrap();
    assert_eq!(root, "# Snapshot policy MYTEXT\n\nkeep 24\n");
}
