//! Starts and talks to `steam-helper` processes (one per game) for the
//! local-Steam-client mode. Protocol: JSON lines over stdin/stdout.
use crate::logbus;
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::oneshot;

type Pending = Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>;
type ExitHook = Arc<dyn Fn(u32, bool) + Send + Sync>;

pub struct HelperProc {
    pub appid: u32,
    #[allow(dead_code)]
    pub pid: u32,
    stdin: tokio::sync::Mutex<ChildStdin>,
    child: tokio::sync::Mutex<Option<Child>>,
    pending: Pending,
    next_id: AtomicU64,
    alive: AtomicBool,
    quitting: AtomicBool,
}

impl HelperProc {
    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    pub async fn request(&self, cmd: &str, args: Value, timeout: Duration) -> Result<Value> {
        if !self.alive() {
            bail!("the helper for app {} is not running", self.appid);
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let line = format!("{}\n", json!({ "id": id, "cmd": cmd, "args": args }));
        {
            let mut stdin = self.stdin.lock().await;
            if let Err(e) = stdin.write_all(line.as_bytes()).await {
                self.pending.lock().unwrap().remove(&id);
                bail!("could not talk to helper: {e}");
            }
            let _ = stdin.flush().await;
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(msg))) => Err(anyhow!(msg)),
            Ok(Err(_)) => Err(anyhow!("helper for app {} exited", self.appid)),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(anyhow!("helper for app {} timed out on '{cmd}'", self.appid))
            }
        }
    }

    async fn shutdown(&self) {
        self.quitting.store(true, Ordering::SeqCst);
        if self.alive() {
            let _ = self.request("quit", Value::Null, Duration::from_millis(1500)).await;
        }
        for _ in 0..15 {
            if !self.alive() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if let Some(child) = self.child.lock().await.as_mut() {
            let _ = child.start_kill();
        }
    }
}

pub struct HelperManager {
    procs: Mutex<HashMap<u32, Arc<HelperProc>>>,
    starting: Mutex<HashSet<u32>>,
    exe: Mutex<PathBuf>,
    lib_dirs: Mutex<Vec<PathBuf>>,
    on_exit: Mutex<Option<ExitHook>>,
}

fn helper_file_name() -> &'static str {
    if cfg!(windows) {
        "steam-helper.exe"
    } else {
        "steam-helper"
    }
}

impl Default for HelperManager {
    fn default() -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_default();
        let exe = std::env::var_os("IDLEKIT_HELPER")
            .map(PathBuf::from)
            .unwrap_or_else(|| exe_dir.join(helper_file_name()));
        Self {
            procs: Mutex::new(HashMap::new()),
            starting: Mutex::new(HashSet::new()),
            exe: Mutex::new(exe),
            lib_dirs: Mutex::new(vec![exe_dir]),
            on_exit: Mutex::new(None),
        }
    }
}

impl HelperManager {
    pub fn add_lib_dir(&self, dir: PathBuf) {
        let mut dirs = self.lib_dirs.lock().unwrap();
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }

    pub fn helper_path(&self) -> PathBuf {
        self.exe.lock().unwrap().clone()
    }

    /// Called with (appid, unexpected) whenever a helper process exits.
    pub fn set_on_exit(&self, hook: ExitHook) {
        *self.on_exit.lock().unwrap() = Some(hook);
    }

    pub fn get(&self, appid: u32) -> Option<Arc<HelperProc>> {
        self.procs.lock().unwrap().get(&appid).filter(|p| p.alive()).cloned()
    }

    /// Return the running helper for `appid`, starting one if needed.
    pub async fn ensure(&self, appid: u32) -> Result<Arc<HelperProc>> {
        loop {
            if let Some(p) = self.get(appid) {
                return Ok(p);
            }
            let claimed = self.starting.lock().unwrap().insert(appid);
            if claimed {
                break;
            }
            // Someone else is starting it; wait for them.
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        let result = self.spawn(appid).await;
        self.starting.lock().unwrap().remove(&appid);
        result
    }

    async fn spawn(&self, appid: u32) -> Result<Arc<HelperProc>> {
        let exe = self.helper_path();
        if !exe.exists() {
            bail!(
                "steam-helper was not found at {} — reinstall IdleKit or build it with `npm run helper`",
                exe.display()
            );
        }
        let mut cmd = Command::new(&exe);
        cmd.arg("--appid")
            .arg(appid.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = exe.parent() {
            cmd.current_dir(dir);
        }
        let dirs = self.lib_dirs.lock().unwrap().clone();
        let var = if cfg!(windows) {
            "PATH"
        } else if cfg!(target_os = "macos") {
            "DYLD_LIBRARY_PATH"
        } else {
            "LD_LIBRARY_PATH"
        };
        let mut paths: Vec<PathBuf> = dirs;
        if let Some(existing) = std::env::var_os(var) {
            paths.extend(std::env::split_paths(&existing));
        }
        if let Ok(joined) = std::env::join_paths(paths) {
            cmd.env(var, joined);
        }
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn().map_err(|e| anyhow!("could not start steam-helper: {e}"))?;
        let pid = child.id().unwrap_or(0);
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let stderr = child.stderr.take();

        let proc = Arc::new(HelperProc {
            appid,
            pid,
            stdin: tokio::sync::Mutex::new(stdin),
            child: tokio::sync::Mutex::new(Some(child)),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            alive: AtomicBool::new(true),
            quitting: AtomicBool::new(false),
        });

        let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();
        let reader_proc = proc.clone();
        let hook = self.on_exit.lock().unwrap().clone();
        tokio::spawn(async move {
            let mut ready_tx = Some(ready_tx);
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    // Steam's own diagnostics sometimes land on stdout.
                    if !line.trim().is_empty() {
                        logbus::debug("helper", format!("[{appid}] {line}"));
                    }
                    continue;
                };
                if let Some(event) = msg.get("event").and_then(Value::as_str) {
                    let message = msg.get("message").and_then(Value::as_str).unwrap_or("").to_string();
                    match event {
                        "ready" => {
                            if let Some(tx) = ready_tx.take() {
                                let _ = tx.send(Ok(()));
                            }
                        }
                        "error" => {
                            if let Some(tx) = ready_tx.take() {
                                let _ = tx.send(Err(message.clone()));
                            }
                            logbus::warn("helper", format!("[{appid}] {message}"));
                        }
                        _ => logbus::debug("helper", format!("[{appid}] {event}: {message}")),
                    }
                    continue;
                }
                if let Some(id) = msg.get("id").and_then(Value::as_u64) {
                    if let Some(tx) = reader_proc.pending.lock().unwrap().remove(&id) {
                        let ok = msg.get("ok").and_then(Value::as_bool).unwrap_or(false);
                        let _ = tx.send(if ok {
                            Ok(msg.get("data").cloned().unwrap_or(Value::Null))
                        } else {
                            Err(msg
                                .get("error")
                                .and_then(Value::as_str)
                                .unwrap_or("helper error")
                                .to_string())
                        });
                    }
                }
            }
            reader_proc.alive.store(false, Ordering::SeqCst);
            for (_, tx) in reader_proc.pending.lock().unwrap().drain() {
                let _ = tx.send(Err("helper exited".into()));
            }
            let status = match reader_proc.child.lock().await.as_mut() {
                Some(child) => child.wait().await.ok(),
                None => None,
            };
            if let Some(tx) = ready_tx.take() {
                let _ = tx.send(Err(match status {
                    Some(s) => format!(
                        "steam-helper exited early ({s}). Is Steam running, signed in, and does this account own app {appid}?"
                    ),
                    None => "steam-helper exited early".into(),
                }));
            }
            let unexpected = !reader_proc.quitting.load(Ordering::SeqCst);
            if let Some(hook) = hook {
                hook(appid, unexpected);
            }
        });
        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.trim().is_empty() {
                        logbus::debug("helper", format!("[{appid}] {line}"));
                    }
                }
            });
        }

        match tokio::time::timeout(Duration::from_secs(25), ready_rx).await {
            Ok(Ok(Ok(()))) => {
                self.procs.lock().unwrap().insert(appid, proc.clone());
                Ok(proc)
            }
            Ok(Ok(Err(msg))) => {
                proc.shutdown().await;
                Err(anyhow!(msg))
            }
            _ => {
                proc.shutdown().await;
                Err(anyhow!("steam-helper for app {appid} did not start in time"))
            }
        }
    }

    pub async fn stop(&self, appid: u32) {
        let proc = self.procs.lock().unwrap().remove(&appid);
        if let Some(p) = proc {
            p.shutdown().await;
        }
    }

    pub async fn stop_all(&self) {
        let all: Vec<Arc<HelperProc>> = self.procs.lock().unwrap().drain().map(|(_, p)| p).collect();
        let futs = all.iter().map(|p| p.shutdown());
        futures_util::future::join_all(futs).await;
    }
}
