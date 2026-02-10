//! fcitx5 D-Bus IME integration
//!
//! Implement Japanese input (IME) via fcitx5 D-Bus interface.
//! D-Bus communication runs in a separate thread (tokio runtime),
//! communicating with main thread via mpsc channel.
//! Works normally even if fcitx5 is not running (fallback).

use anyhow::{anyhow, Result};
use log::{debug, info, warn};
use std::sync::mpsc;

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
    let connection = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            let _ = ready_tx.send(Err(anyhow!("Failed to connect to D-Bus session bus: {}", e)));
            return Ok(());
        }
    };

    // fcitx5 Controller proxy
    let controller = match FcitxInputMethodProxy::new(&connection).await {
        Ok(c) => c,
        Err(e) => {
            let _ = ready_tx.send(Err(anyhow!("Failed to connect to fcitx5 InputMethod: {}", e)));
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
