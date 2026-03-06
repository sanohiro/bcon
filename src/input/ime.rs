//! fcitx5 D-Bus IME integration
//!
//! Implement Japanese input (IME) via fcitx5 D-Bus interface.
//! D-Bus communication runs in a separate thread (tokio runtime),
//! communicating with main thread via mpsc channel.
//! Works normally even if fcitx5 is not running (fallback).

use anyhow::{anyhow, Result};
use log::{debug, info, warn};
use std::sync::mpsc;

/// Ensure D-Bus session bus and fcitx5 are available.
///
/// On headless systems (e.g. Ubuntu Server without GUI), there is no D-Bus
/// session bus running by default. This function detects the situation and
/// automatically starts `dbus-daemon` and `fcitx5` so that IME works
/// out of the box.
///
/// Uses a well-known socket path (`/run/user/$UID/bcon-dbus` or `/tmp/bcon-dbus-$UID`)
/// so that dbus-daemon survives bcon restarts and can be reused.
///
/// When running as root (systemd service), uses a custom D-Bus config that
/// allows cross-user access, and writes `/etc/profile.d/bcon-dbus.sh` so that
/// login shells get DBUS_SESSION_BUS_ADDRESS (survives /bin/login clearenv).
///
/// Must be called from the main thread before any IME threads are spawned.
pub fn ensure_ime_environment() {
    // 1. Ensure D-Bus session bus is available
    if std::env::var("DBUS_SESSION_BUS_ADDRESS").is_ok() {
        // Already set (e.g. desktop session) — use as-is
        return;
    }

    // Determine a stable socket path for bcon's D-Bus session
    let uid = unsafe { libc::getuid() };
    let socket_path = {
        let xdg_dir = format!("/run/user/{}", uid);
        if std::path::Path::new(&xdg_dir).is_dir() {
            format!("{}/bcon-dbus", xdg_dir)
        } else {
            format!("/tmp/bcon-dbus-{}", uid)
        }
    };
    let addr = format!("unix:path={}", socket_path);

    // Check if our previously-started dbus-daemon is still alive
    if std::path::Path::new(&socket_path).exists() {
        // Verify socket ownership before reuse (prevent hijacking)
        let mut reuse = false;
        match std::fs::symlink_metadata(&socket_path) {
            Ok(meta) => {
                use std::os::unix::fs::{FileTypeExt, MetadataExt};
                if meta.uid() != uid {
                    warn!(
                        "IME: socket {} owned by uid {}, expected {}, removing",
                        socket_path,
                        meta.uid(),
                        uid
                    );
                    let _ = std::fs::remove_file(&socket_path);
                } else if !meta.file_type().is_socket() {
                    warn!(
                        "IME: {} is not a socket (type mismatch), removing",
                        socket_path
                    );
                    let _ = std::fs::remove_file(&socket_path);
                } else {
                    reuse = true;
                }
            }
            Err(e) => {
                warn!("IME: cannot stat socket {}: {}, removing", socket_path, e);
                let _ = std::fs::remove_file(&socket_path);
            }
        }
        if reuse {
            std::env::set_var("DBUS_SESSION_BUS_ADDRESS", &addr);
            info!("IME: reusing existing D-Bus session: {}", addr);
            // When root, ensure socket is accessible and profile.d script exists
            if uid == 0 {
                make_socket_accessible(&socket_path);
                write_profile_d_script(&socket_path);
            }
        }
    }

    if std::env::var("DBUS_SESSION_BUS_ADDRESS").is_err() {
        if uid == 0 {
            // Running as root (e.g., systemd service):
            // Standard --session D-Bus only allows the owner's UID to connect.
            // Use custom config with EXTERNAL+ANONYMOUS auth so the logged-in
            // user's fcitx5 can also connect to bcon's D-Bus.
            start_dbus_cross_user(&socket_path, &addr);
        } else {
            // Running as normal user: standard session bus
            info!("IME: DBUS_SESSION_BUS_ADDRESS not set, starting dbus-daemon...");
            match std::process::Command::new("dbus-daemon")
                .args([
                    "--session",
                    "--fork",
                    "--print-address=1",
                    &format!("--address={}", addr),
                ])
                .output()
            {
                Ok(output) if output.status.success() => {
                    std::env::set_var("DBUS_SESSION_BUS_ADDRESS", &addr);
                    info!("IME: started D-Bus session daemon: {}", addr);
                }
                Ok(output) => {
                    warn!(
                        "IME: dbus-daemon failed: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                    return;
                }
                Err(e) => {
                    info!("IME: dbus-daemon not available: {}", e);
                    return;
                }
            }
        }
    }

    // 2. Ensure fcitx5 is running on OUR bus
    // Note: don't use pgrep — it may find fcitx5 on a different D-Bus session.
    // When root, fcitx5 will be started by the user's login shell via profile.d.
    start_fcitx5();
}

/// Start D-Bus daemon with cross-user access (for root/systemd case).
///
/// Standard session bus only allows the owner's UID to connect. When bcon runs
/// as root via systemd, the logged-in user (different UID) needs to run fcitx5
/// on the same D-Bus. This uses a custom config with permissive auth policy.
///
/// Also writes `/etc/profile.d/bcon-dbus.sh` so the user's login shell gets
/// DBUS_SESSION_BUS_ADDRESS set (survives /bin/login's clearenv()).
fn start_dbus_cross_user(socket_path: &str, addr: &str) {
    let config_path = "/tmp/bcon-dbus-config.xml";
    let config = format!(
        r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <listen>unix:path={socket}</listen>
  <auth>EXTERNAL</auth>
  <auth>ANONYMOUS</auth>
  <allow_anonymous/>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>"#,
        socket = socket_path,
    );

    if let Err(e) = std::fs::write(config_path, &config) {
        warn!("IME: failed to write D-Bus config {}: {}", config_path, e);
        return;
    }

    info!("IME: starting D-Bus daemon (cross-user mode)...");
    match std::process::Command::new("dbus-daemon")
        .args(["--config-file", config_path, "--fork", "--print-address=1"])
        .output()
    {
        Ok(output) if output.status.success() => {
            std::env::set_var("DBUS_SESSION_BUS_ADDRESS", addr);
            info!("IME: started D-Bus daemon (cross-user): {}", addr);
            make_socket_accessible(socket_path);
            write_profile_d_script(socket_path);
        }
        Ok(output) => {
            warn!(
                "IME: dbus-daemon failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Err(e) => {
            info!("IME: dbus-daemon not available: {}", e);
        }
    }
}

/// Make D-Bus socket accessible to all users (chmod 666).
///
/// Needed when root starts dbus-daemon — the socket is created with root
/// ownership, but the logged-in user needs to connect (for fcitx5).
fn make_socket_accessible(socket_path: &str) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o666)) {
        warn!("IME: failed to chmod socket {}: {}", socket_path, e);
    }
}

/// Write `/etc/profile.d/bcon-dbus.sh` for login shell D-Bus integration.
///
/// When bcon runs as root via systemd, `/bin/login` calls `clearenv()` which
/// clears ALL environment variables. This profile.d script ensures login shells
/// get `DBUS_SESSION_BUS_ADDRESS` set before `.bashrc` runs, so fcitx5
/// connects to bcon's D-Bus session.
///
/// The script also starts fcitx5 if available, since bcon (root) cannot
/// start fcitx5 itself (fcitx5 crashes under root).
fn write_profile_d_script(socket_path: &str) {
    let profile_d = "/etc/profile.d";
    if !std::path::Path::new(profile_d).is_dir() {
        warn!(
            "IME: {} does not exist, skipping profile.d script",
            profile_d
        );
        return;
    }

    let script = format!(
        r#"# Auto-generated by bcon for IME D-Bus integration - DO NOT EDIT
# Sets DBUS_SESSION_BUS_ADDRESS for login shells and starts fcitx5.
# This is needed because /bin/login calls clearenv(), losing all env vars.
if [ -S "{socket}" ] && [ -z "$DBUS_SESSION_BUS_ADDRESS" ]; then
    export DBUS_SESSION_BUS_ADDRESS="unix:path={socket}"
    if command -v fcitx5 >/dev/null 2>&1; then
        fcitx5 -d 2>/dev/null || true
    fi
fi
"#,
        socket = socket_path,
    );

    let script_path = format!("{}/bcon-dbus.sh", profile_d);
    match std::fs::write(&script_path, &script) {
        Ok(()) => {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o644));
            info!("IME: wrote {} (socket: {})", script_path, socket_path);
        }
        Err(e) => {
            warn!("IME: failed to write {}: {}", script_path, e);
        }
    }
}

/// Start fcitx5 daemon on the current DBUS_SESSION_BUS_ADDRESS.
///
/// Safe to call multiple times — fcitx5 -d exits if already running on the same bus.
/// Skips when running as root (uid=0) since fcitx5 crashes under root.
pub fn start_fcitx5() {
    if unsafe { libc::getuid() } == 0 {
        // fcitx5 cannot run as root — it will be started by the user's shell
        // (.bashrc) which inherits DBUS_SESSION_BUS_ADDRESS from bcon.
        debug!("IME: skipping fcitx5 start (running as root, user shell will start it)");
        return;
    }
    info!("IME: starting fcitx5 on bcon D-Bus...");
    match std::process::Command::new("fcitx5").arg("-d").spawn() {
        Ok(_) => {
            info!("IME: started fcitx5 daemon, waiting for initialization...");
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        Err(e) => {
            info!("IME: fcitx5 not available: {}", e);
        }
    }
}

/// Get the current bcon D-Bus address if one was set up.
///
/// Returns the address string for passing to child processes via extra_env.
pub fn dbus_address() -> Option<String> {
    std::env::var("DBUS_SESSION_BUS_ADDRESS").ok()
}

// === Type definitions ===

/// Events from IME to main thread
pub enum ImeEvent {
    /// Committed string
    Commit(String),
    /// Preedit update
    Preedit {
        segments: Vec<PreeditSegment>,
        cursor: i32,
    },
    /// Preedit clear
    PreeditClear,
    /// Key not processed by IME (convert in main thread and send to PTY)
    #[allow(dead_code)]
    ForwardKey {
        keysym: u32,
        state: u32,
        is_release: bool,
    },
    /// Candidate list update
    UpdateCandidates(CandidateState),
    /// Clear candidates
    ClearCandidates,
}

/// Candidate state
#[allow(dead_code)]
pub struct CandidateState {
    /// Candidate list (label, text)
    pub candidates: Vec<(String, String)>,
    /// Selected candidate index
    pub selected_index: i32,
    /// Layout hint: 0=unspecified, 1=vertical, 2=horizontal
    pub layout_hint: i32,
    /// Has previous page
    pub has_prev: bool,
    /// Has next page
    pub has_next: bool,
}

/// Preedit segment
pub struct PreeditSegment {
    pub text: String,
    /// 0=normal, 1=highlighted (conversion target)
    pub format: i32,
}

/// Preedit state held by main thread
pub struct PreeditState {
    pub segments: Vec<PreeditSegment>,
    pub cursor: i32,
}

impl PreeditState {
    pub fn new() -> Self {
        Self {
            segments: vec![],
            cursor: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn clear(&mut self) {
        self.segments.clear();
        self.cursor = 0;
    }

    /// Return full preedit text
    #[allow(dead_code)]
    pub fn text(&self) -> String {
        self.segments.iter().map(|s| s.text.as_str()).collect()
    }
}

/// Key event from main thread to IME thread
pub struct ImeKeyEvent {
    pub keysym: u32,
    pub keycode: u32,
    pub state: u32,
    pub is_release: bool,
}

// === zbus proxy definitions ===

#[zbus::proxy(
    interface = "org.fcitx.Fcitx.InputMethod1",
    default_service = "org.fcitx.Fcitx5",
    default_path = "/org/freedesktop/portal/inputmethod"
)]
trait FcitxInputMethod {
    fn create_input_context(
        &self,
        args: Vec<(String, String)>,
    ) -> zbus::Result<(zbus::zvariant::OwnedObjectPath, Vec<u8>)>;
}

#[zbus::proxy(
    interface = "org.fcitx.Fcitx.InputContext1",
    default_service = "org.fcitx.Fcitx5"
)]
trait FcitxInputContext {
    fn process_key_event(
        &self,
        keysym: u32,
        keycode: u32,
        state: u32,
        is_release: bool,
        time: u32,
    ) -> zbus::Result<bool>;

    fn focus_in(&self) -> zbus::Result<()>;
    fn focus_out(&self) -> zbus::Result<()>;
    fn reset(&self) -> zbus::Result<()>;
    fn set_capability(&self, cap: u64) -> zbus::Result<()>;
    fn set_cursor_rect(&self, x: i32, y: i32, w: i32, h: i32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn commit_string(&self, text: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    fn update_formatted_preedit(
        &self,
        preedit: Vec<(String, i32)>,
        cursor_pos: i32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    fn forward_key(&self, keysym: u32, state: u32, is_release: bool) -> zbus::Result<()>;

    #[zbus(signal, name = "UpdateClientSideUI")]
    fn update_client_side_ui(
        &self,
        preedit: Vec<(String, i32)>,
        cursor_pos: i32,
        aux_up: Vec<(String, i32)>,
        aux_down: Vec<(String, i32)>,
        candidates: Vec<(String, String)>,
        candidate_index: i32,
        layout_hint: i32,
        has_prev: bool,
        has_next: bool,
    ) -> zbus::Result<()>;
}

// === ImeClient ===

/// fcitx5 IME client
///
/// Held by main thread, sends key events and polls IME events.
pub struct ImeClient {
    /// IME event receiver channel
    event_rx: mpsc::Receiver<ImeEvent>,
    /// Key event sender channel
    key_tx: tokio::sync::mpsc::Sender<ImeKeyEvent>,
    /// IME thread (for join, automatically terminates on drop)
    _thread: std::thread::JoinHandle<()>,
}

impl ImeClient {
    /// Connect to fcitx5 and create ImeClient
    ///
    /// Returns Err if fcitx5 is not running or D-Bus connection fails.
    /// 3 second timeout.
    pub fn try_new() -> Result<Self> {
        let (event_tx, event_rx) = mpsc::channel::<ImeEvent>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let (key_tx, key_rx) = tokio::sync::mpsc::channel::<ImeKeyEvent>(64);

        let thread = std::thread::Builder::new()
            .name("bcon-ime".into())
            .spawn(move || {
                ime_thread(event_tx, key_rx, ready_tx);
            })
            .map_err(|e| anyhow!("Failed to start IME thread: {}", e))?;

        // Wait for connection (3 second timeout)
        match ready_rx.recv_timeout(std::time::Duration::from_secs(3)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(anyhow!("fcitx5 connection timeout")),
        }

        Ok(Self {
            event_rx,
            key_tx,
            _thread: thread,
        })
    }

    /// Send key event to IME (non-blocking)
    ///
    /// Returns false if send fails (e.g., IME thread has terminated).
    pub fn send_key(&self, event: ImeKeyEvent) -> bool {
        self.key_tx.try_send(event).is_ok()
    }

    /// Get all pending IME events
    pub fn poll_events(&self) -> Vec<ImeEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }
}

/// IME thread main function
fn ime_thread(
    event_tx: mpsc::Sender<ImeEvent>,
    key_rx: tokio::sync::mpsc::Receiver<ImeKeyEvent>,
    ready_tx: mpsc::Sender<Result<()>>,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            let _ = ready_tx.send(Err(anyhow!("Failed to create tokio runtime: {}", e)));
            return;
        }
    };

    rt.block_on(async move {
        match ime_async_main(event_tx, key_rx, ready_tx).await {
            Ok(()) => info!("IME thread terminated normally"),
            Err(e) => warn!("IME thread error: {}", e),
        }
    });
}

/// IME thread async main
async fn ime_async_main(
    event_tx: mpsc::Sender<ImeEvent>,
    mut key_rx: tokio::sync::mpsc::Receiver<ImeKeyEvent>,
    ready_tx: mpsc::Sender<Result<()>>,
) -> Result<()> {
    // Connect to D-Bus session bus
    let dbus_addr = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
    info!(
        "IME: connecting to D-Bus session (addr={})",
        if dbus_addr.is_empty() {
            "<not set>"
        } else {
            &dbus_addr
        }
    );
    let connection = match zbus::Connection::session().await {
        Ok(c) => {
            info!("IME: D-Bus session connected");
            c
        }
        Err(e) => {
            let msg = if dbus_addr.is_empty() {
                format!(
                    "Failed to connect to D-Bus session bus (DBUS_SESSION_BUS_ADDRESS not set): {}",
                    e
                )
            } else {
                format!(
                    "Failed to connect to D-Bus session bus (addr={}): {}",
                    dbus_addr, e
                )
            };
            let _ = ready_tx.send(Err(anyhow!(msg)));
            return Ok(());
        }
    };

    // fcitx5 Controller proxy
    info!("IME: connecting to fcitx5 InputMethod...");
    let controller = match FcitxInputMethodProxy::new(&connection).await {
        Ok(c) => {
            info!("IME: fcitx5 InputMethod proxy connected");
            c
        }
        Err(e) => {
            let _ = ready_tx.send(Err(anyhow!(
                "Failed to connect to fcitx5 InputMethod (is fcitx5 running?): {}",
                e
            )));
            return Ok(());
        }
    };

    // Create InputContext
    let args = vec![("program".to_string(), "bcon".to_string())];

    let (ic_path, _) = match controller.create_input_context(args).await {
        Ok(result) => result,
        Err(e) => {
            let _ = ready_tx.send(Err(anyhow!("Failed to create InputContext: {}", e)));
            return Ok(());
        }
    };

    debug!("InputContext path: {}", ic_path);

    // Create InputContext proxy
    let ic = match FcitxInputContextProxy::builder(&connection)
        .path(ic_path)?
        .build()
        .await
    {
        Ok(ic) => ic,
        Err(e) => {
            let _ = ready_tx.send(Err(anyhow!("Failed to create InputContext proxy: {}", e)));
            return Ok(());
        }
    };

    // Declare preedit + ClientSideInputPanel support
    // bit 1 = preedit, bit 39 = ClientSideInputPanel
    if let Err(e) = ic.set_capability(0x2 | (1u64 << 39)).await {
        warn!("SetCapability failed (continuing): {}", e);
    }

    // Focus in
    if let Err(e) = ic.focus_in().await {
        warn!("FocusIn failed (continuing): {}", e);
    }

    // Get signal streams
    let mut commit_stream = ic.receive_commit_string().await?;
    let mut preedit_stream = ic.receive_update_formatted_preedit().await?;
    let mut forward_stream = ic.receive_forward_key().await?;
    let mut candidate_stream = ic.receive_update_client_side_ui().await?;

    // Notify connection success
    let _ = ready_tx.send(Ok(()));
    info!("fcitx5 IME thread started");

    // Event loop
    loop {
        tokio::select! {
            // CommitString signal
            Some(signal) = async { commit_stream.next().await } => {
                match signal.args() {
                    Ok(args) => {
                        let text = args.text().to_string();
                        debug!("IME CommitString: {:?}", text);
                        let _ = event_tx.send(ImeEvent::Commit(text));
                    }
                    Err(e) => warn!("CommitString parse error: {}", e),
                }
            }

            // UpdateFormattedPreedit signal
            Some(signal) = async { preedit_stream.next().await } => {
                match signal.args() {
                    Ok(args) => {
                        let preedit_data = args.preedit();
                        let cursor = *args.cursor_pos();

                        if preedit_data.is_empty() {
                            debug!("IME PreeditClear");
                            let _ = event_tx.send(ImeEvent::PreeditClear);
                        } else {
                            let segments: Vec<PreeditSegment> = preedit_data
                                .iter()
                                .map(|(text, format)| PreeditSegment {
                                    text: text.clone(),
                                    format: *format,
                                })
                                .collect();
                            debug!("IME Preedit: {:?} cursor={}",
                                segments.iter().map(|s| &s.text).collect::<Vec<_>>(), cursor);
                            let _ = event_tx.send(ImeEvent::Preedit { segments, cursor });
                        }
                    }
                    Err(e) => warn!("UpdateFormattedPreedit parse error: {}", e),
                }
            }

            // ForwardKey signal
            Some(signal) = async { forward_stream.next().await } => {
                match signal.args() {
                    Ok(args) => {
                        let keysym = *args.keysym();
                        let state = *args.state();
                        let is_release = *args.is_release();
                        debug!("IME ForwardKey: keysym={:#x} state={:#x} release={}",
                            keysym, state, is_release);
                        let _ = event_tx.send(ImeEvent::ForwardKey {
                            keysym,
                            state,
                            is_release,
                        });
                    }
                    Err(e) => warn!("ForwardKey parse error: {}", e),
                }
            }

            // UpdateClientSideUI signal (candidates)
            Some(signal) = async { candidate_stream.next().await } => {
                match signal.args() {
                    Ok(args) => {
                        let candidates = args.candidates();
                        let candidate_index = *args.candidate_index();
                        let layout_hint = *args.layout_hint();
                        let has_prev = *args.has_prev();
                        let has_next = *args.has_next();

                        if candidates.is_empty() {
                            debug!("IME ClearCandidates");
                            let _ = event_tx.send(ImeEvent::ClearCandidates);
                        } else {
                            let cands: Vec<(String, String)> = candidates
                                .iter()
                                .map(|(label, text)| (label.clone(), text.clone()))
                                .collect();
                            debug!("IME UpdateCandidates: {} items, selected={}",
                                cands.len(), candidate_index);
                            let _ = event_tx.send(ImeEvent::UpdateCandidates(CandidateState {
                                candidates: cands,
                                selected_index: candidate_index,
                                layout_hint,
                                has_prev,
                                has_next,
                            }));
                        }
                    }
                    Err(e) => warn!("UpdateClientSideUI parse error: {}", e),
                }
            }

            // Key event from main thread
            Some(key_event) = key_rx.recv() => {
                match ic.process_key_event(
                    key_event.keysym,
                    key_event.keycode,
                    key_event.state,
                    key_event.is_release,
                    0, // time
                ).await {
                    Ok(handled) => {
                        if !handled && !key_event.is_release {
                            // Not processed by IME -> notify as ForwardKey
                            debug!("IME unprocessed key: keysym={:#x}", key_event.keysym);
                            let _ = event_tx.send(ImeEvent::ForwardKey {
                                keysym: key_event.keysym,
                                state: key_event.state,
                                is_release: false,
                            });
                        }
                    }
                    Err(e) => {
                        // ProcessKeyEvent error -> pass through
                        warn!("ProcessKeyEvent error: {}", e);
                        if !key_event.is_release {
                            let _ = event_tx.send(ImeEvent::ForwardKey {
                                keysym: key_event.keysym,
                                state: key_event.state,
                                is_release: false,
                            });
                        }
                    }
                }
            }

            // All channels closed
            else => {
                info!("IME event loop terminated");
                break;
            }
        }
    }

    Ok(())
}

// Required to use next() on zbus SignalStream
use futures_util::StreamExt;
