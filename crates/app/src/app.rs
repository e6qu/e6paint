//! The canvas application: document, camera, tool gestures, and per-frame painting.

#[cfg(target_arch = "wasm32")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use web_time::{Duration, Instant};

#[cfg(target_arch = "wasm32")]
use e6_collab::{self, Backoff, ClientIdentity, Outbox, SendThrottle, frame, presence_json};
use e6_doc::{BoardDoc, ElementSpec};
use e6_editor::{Selection, Tool};
use e6_render::build_display_list;
use e6_scene::{
    Aabb, Camera, Element, ElementId, ElementKind, MAX_ZOOM, MIN_ZOOM, Point, Size, welcome_board,
};
use e6_ui::{
    DrawerTab, HudAvailability, HudCommand, HudState, draw_canvas_text, draw_status_bar,
    draw_tool_bar, draw_top_bar,
};
use vello::Scene;

use crate::gfx::{FrameJob, Gfx, VelloCallback};
use crate::storage;

/// Exponential-moving-average weight for the frame-time readout.
const FRAME_EMA_WEIGHT: f32 = 0.1;

/// Default ink color for drawn content, `0xAARRGGBB`.
const INK: u32 = 0xFF1D_1D2B;
/// Sticky note fill.
const STICKY_FILL: u32 = 0xFFF7_E97B;
/// Default world-unit sizes for placed text and sticky notes.
const TEXT_PLACE_SIZE: f32 = 240.0;
const STICKY_SIZE: f32 = 220.0;
/// Screen-pixel distance before a pen sample is recorded.
const PEN_SAMPLE_PX: f32 = 3.0;
/// Screen-pixel movement below which a press-release counts as a click.
const CLICK_SLOP_PX: f32 = 4.0;
/// New frame size in world units.
const FRAME_SIZE: f32 = 640.0;
/// Camera tween duration for present-mode transitions.
const PRESENT_TWEEN: Duration = Duration::from_millis(380);

/// Relay HTTP base when the page carries no `?relay=` override (local dev).
#[cfg(target_arch = "wasm32")]
const RELAY_HTTP_BASE: &str = "http://127.0.0.1:8480";

/// A remote participant's cursor.
#[derive(Debug, Clone)]
#[cfg(target_arch = "wasm32")]
struct RemoteCursor {
    peer: String,
    name: String,
    color: u32,
    at: Point,
}

/// A live collaboration session's client-side state.
#[cfg(target_arch = "wasm32")]
struct CollabSession {
    identity: ClientIdentity,
    room: crate::collab_ws::RoomConnection,
    transport: Option<crate::collab_ws::Transport>,
    /// HTTP base for reconnects (the WS URL derives from it).
    relay_http_base: String,
    outbox: Outbox,
    presence_throttle: SendThrottle,
    backoff: Backoff,
    /// When due, retry the socket connection.
    reconnect_at: Option<Instant>,
    /// The relay told us this token is view-only: edits stay local and are not
    /// synced (the relay would reject them anyway).
    read_only: bool,
    /// Liveness: when the last ping went out and when the last pong came back.
    /// A ping without a pong inside the timeout means the transport is dead.
    last_ping_sent: Option<Instant>,
    last_pong_at: Option<Instant>,
    remote_cursors: Vec<RemoteCursor>,
}

/// An in-progress creation gesture.
enum Draft {
    Pen {
        points: Vec<Point>,
        last_screen: Point,
    },
    Shape {
        kind: ElementKind,
        start: Point,
        end: Point,
    },
    Connector {
        start: Point,
        end: Point,
    },
}

impl Draft {
    /// The element the draft would create right now (preview or final).
    fn to_spec(&self) -> ElementSpec {
        match self {
            Draft::Pen { points, .. } => {
                let bounds = Aabb::from_points(points).unwrap_or(Aabb {
                    min: Point::ZERO,
                    max: Point::new(1.0, 1.0),
                });
                ElementSpec::simple(ElementKind::Stroke, bounds, INK)
                    .outlined(INK, 2.5)
                    .with_points(points.clone())
            }
            Draft::Shape { kind, start, end } => {
                ElementSpec::simple(*kind, rect_between(*start, *end), 0x0000_0000)
                    .outlined(INK, 2.0)
            }
            Draft::Connector { start, end } => {
                let bounds = Aabb::from_points(&[*start, *end]).unwrap_or(Aabb {
                    min: Point::ZERO,
                    max: Point::new(1.0, 1.0),
                });
                ElementSpec::simple(ElementKind::Connector, bounds, INK)
                    .outlined(INK, 2.0)
                    .with_points(vec![*start, *end])
            }
        }
    }
}

fn rect_between(a: Point, b: Point) -> Aabb {
    Aabb {
        min: Point::new(a.x.min(b.x), a.y.min(b.y)),
        max: Point::new(a.x.max(b.x), a.y.max(b.y)),
    }
}

/// An in-progress move gesture over the selection.
struct DragState {
    last_world: Point,
}

/// An in-progress marquee selection (world-space corners).
struct MarqueeState {
    start: Point,
    end: Point,
}

/// An animated camera transition.
struct CameraTween {
    from: Camera,
    to: Camera,
    started: Instant,
}

impl CameraTween {
    fn camera_now(&self) -> Camera {
        let t = (Instant::now().duration_since(self.started).as_secs_f32()
            / PRESENT_TWEEN.as_secs_f32())
        .clamp(0.0, 1.0);
        let ease = t * t * (3.0 - 2.0 * t);
        Camera {
            origin: Point::new(
                self.from.origin.x + (self.to.origin.x - self.from.origin.x) * ease,
                self.from.origin.y + (self.to.origin.y - self.from.origin.y) * ease,
            ),
            zoom: (self.from.zoom.ln() + (self.to.zoom.ln() - self.from.zoom.ln()) * ease).exp(),
        }
    }

    fn finished(&self) -> bool {
        Instant::now().duration_since(self.started) >= PRESENT_TWEEN
    }
}

impl MarqueeState {
    fn bounds(&self) -> Aabb {
        rect_between(self.start, self.end)
    }
}

/// Root application state.
/// The async OPFS restore's verdict: a usable board, or an empty store (which
/// seeds the demo). Read errors leave the slot empty — the board starts fresh.
#[cfg(target_arch = "wasm32")]
enum RestoreOutcome {
    Restored(BoardDoc),
    Empty,
}

pub struct E6App {
    doc: BoardDoc,
    selection: Selection,
    tool: Tool,
    drag: Option<DragState>,
    marquee: Option<MarqueeState>,
    draft: Option<Draft>,
    editing: Option<ElementId>,
    editing_focused: bool,
    /// Present mode: index into [`BoardDoc::frames`].
    present: Option<usize>,
    /// Animated camera transition (present mode and future fit animations).
    camera_tween: Option<CameraTween>,
    camera: Camera,
    gfx: Arc<Mutex<Gfx>>,
    frame_job: Arc<Mutex<Option<FrameJob>>>,
    hud: HudState,
    last_frame: Option<Instant>,
    /// Set once the first egui frame has run; announces readiness for e2e.
    painted_once: bool,
    /// Opened via a share link: content belongs to the room, so the local demo
    /// seed, restore, and autosave stay out of the way.
    room_first: bool,
    /// Where rooms are created and joined (`?relay=` override or the default).
    #[cfg(target_arch = "wasm32")]
    relay_http_base: String,
    fitted: bool,
    /// Which right-drawer tab is active.
    drawer_tab: DrawerTab,
    /// Canvas viewport in physical pixels, remembered for present-mode fitting.
    last_viewport: Size,
    /// Canvas rect in egui points; anchor for the zoom widget.
    last_canvas_rect: Option<egui::Rect>,
    /// A committed gesture is not yet autosaved.
    dirty: bool,
    /// mol code panel state (source text + last expansion errors).
    code_panel: e6_ui::CodePanelState,
    /// Source text of the last mol expansion (drives re-apply).
    mol_applied: Option<String>,
    /// Live collaboration session state (web only).
    #[cfg(target_arch = "wasm32")]
    collab: Option<CollabSession>,
    /// Web-only: in-flight join flow + socket events inbox.
    #[cfg(target_arch = "wasm32")]
    join_flow: Option<crate::collab_ws::JoinFlow>,
    /// Web-only: socket events inbox shared by the join flow and reconnects.
    #[cfg(target_arch = "wasm32")]
    ws_events: Arc<Mutex<Vec<crate::collab_ws::WsEvent>>>,
    /// Web-only: gate so at most one OPFS save runs at a time.
    #[cfg(target_arch = "wasm32")]
    save_inflight: Arc<AtomicBool>,
    /// Web-only: inbox for the async restore task (OPFS reads cannot block startup).
    /// `Empty` is distinct from an error: only an empty store seeds the demo.
    #[cfg(target_arch = "wasm32")]
    pending_restore: Arc<Mutex<Option<RestoreOutcome>>>,
    /// Web-only: bytes of the image the file picker handed us, drained each frame.
    #[cfg(target_arch = "wasm32")]
    pending_image: Arc<Mutex<Option<Vec<u8>>>>,
    /// Web-only: persistent hidden `<input type=file>`; the closure lives as long as it.
    #[cfg(target_arch = "wasm32")]
    file_input: Option<web_sys::HtmlInputElement>,
    #[cfg(target_arch = "wasm32")]
    #[allow(dead_code)]
    file_closure: Option<wasm_bindgen::closure::Closure<dyn FnMut()>>,
}

/// Pointer position in canvas-local physical pixels: the camera's screen space
/// starts at the canvas rect's top-left, which sits inside the egui panels. Every
/// pointer→world conversion must go through this, or selections and clicks land
/// offset by the panel footprint.
fn canvas_pos(response: &egui::Response, pos: egui::Pos2, pixels_per_point: f32) -> Point {
    canvas_pos_in(response.rect, pos, pixels_per_point)
}

fn canvas_pos_in(rect: egui::Rect, pos: egui::Pos2, pixels_per_point: f32) -> Point {
    let local = pos - rect.min;
    Point::new(local.x * pixels_per_point, local.y * pixels_per_point)
}

impl E6App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, String> {
        // eframe paints through wgpu only (glow is not enabled), so a missing render
        // state means startup already failed upstream; refuse loudly rather than
        // degrade into a canvas that could never be drawn.
        let render_state = cc
            .wgpu_render_state
            .as_ref()
            .ok_or_else(|| "eframe wgpu render state unavailable; refusing to start".to_owned())?;
        let gfx = Arc::new(Mutex::new(Gfx::new(
            &render_state.device,
            render_state.target_format,
        )));
        let mut app = Self {
            doc: BoardDoc::new(),
            selection: Selection::new(),
            tool: Tool::Select,
            drag: None,
            marquee: None,
            draft: None,
            editing: None,
            editing_focused: false,
            present: None,
            camera_tween: None,
            camera: Camera::default(),
            gfx,
            frame_job: Arc::new(Mutex::new(None)),
            hud: HudState {
                title: "Untitled board".to_owned(),
                zoom: 1.0,
                visible_elements: 0,
                total_elements: 0,
                selected_elements: 0,
                frame_ms: 0.0,
                notice: None,
                notice_until: None,
            },
            last_frame: None,
            painted_once: false,
            room_first: false,
            #[cfg(target_arch = "wasm32")]
            relay_http_base: RELAY_HTTP_BASE.to_owned(),
            fitted: false,
            drawer_tab: DrawerTab::Library,
            last_viewport: Size::new(1.0, 1.0),
            last_canvas_rect: None,
            dirty: false,
            code_panel: e6_ui::CodePanelState::default(),
            mol_applied: None,
            #[cfg(target_arch = "wasm32")]
            collab: None,
            #[cfg(target_arch = "wasm32")]
            join_flow: None,
            #[cfg(target_arch = "wasm32")]
            ws_events: Arc::new(Mutex::new(Vec::new())),
            #[cfg(target_arch = "wasm32")]
            save_inflight: Arc::new(AtomicBool::new(false)),
            #[cfg(target_arch = "wasm32")]
            pending_restore: Arc::new(Mutex::new(None)),
            #[cfg(target_arch = "wasm32")]
            pending_image: Arc::new(Mutex::new(None)),
            #[cfg(target_arch = "wasm32")]
            file_input: None,
            #[cfg(target_arch = "wasm32")]
            file_closure: None,
        };
        #[cfg(target_arch = "wasm32")]
        let room_first = app.join_from_query_if_present();
        #[cfg(not(target_arch = "wasm32"))]
        let room_first = false;
        app.room_first = room_first;
        if !room_first {
            // A room-first session takes its content from the room: no demo seed
            // and no local board restore may interfere with it. Local web sessions
            // seed from the restore verdict (Empty) so a saved board never flashes
            // the demo first.
            #[cfg(not(target_arch = "wasm32"))]
            app.restore_native();
            #[cfg(target_arch = "wasm32")]
            app.restore_web();
            #[cfg(not(target_arch = "wasm32"))]
            app.load_demo_board_if_unseeded();
        }
        #[cfg(target_arch = "wasm32")]
        app.setup_file_picker();
        Ok(app)
    }

    /// Share-link startup: `?room=…&token=…` joins directly, skipping the demo seed.
    #[cfg(target_arch = "wasm32")]
    /// Returns true when the page was opened via a share link: the session is then
    /// room-first — board content comes from the room, never from the local demo
    /// seed, restore, or autosave slot (which belong to whatever board was local
    /// before, and must not leak into other people's rooms).
    #[cfg(target_arch = "wasm32")]
    fn join_from_query_if_present(&mut self) -> bool {
        let Some(search) =
            web_sys::window().map(|window| window.location().search().unwrap_or_default())
        else {
            return false;
        };
        self.relay_http_base =
            Self::relay_base_override(&search).unwrap_or(RELAY_HTTP_BASE.to_owned());
        if let Some(link) = e6_collab::parse_share_query(&search) {
            log::info!("share link detected; joining room {}", link.room);
            self.notify("joining room...");
            self.join_flow = Some(crate::collab_ws::JoinFlow::connect_to(
                &self.relay_http_base,
                &link.room,
                &link.token,
            ));
            return true;
        }
        false
    }

    /// Validated `?relay=` override: http(s) only, no trailing slash. An invalid
    /// value is reported and dropped rather than silently coerced.
    #[cfg(target_arch = "wasm32")]
    fn relay_base_override(search: &str) -> Option<String> {
        let base = e6_collab::relay_override(search)?;
        let base = base.trim_end_matches('/').to_owned();
        if base.starts_with("http://") || base.starts_with("https://") {
            Some(base)
        } else {
            log::error!("?relay=`{base}` is not an http(s) base; using the default relay");
            None
        }
    }

    /// Creates the hidden file input once and wires its change handler to the inbox.
    #[cfg(target_arch = "wasm32")]
    fn setup_file_picker(&mut self) {
        use wasm_bindgen::JsCast;

        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            log::error!("no document; image picking is unavailable");
            return;
        };
        let input = match document
            .create_element("input")
            .map_err(|err| format!("{err:?}"))
            .and_then(|element| {
                element
                    .dyn_into::<web_sys::HtmlInputElement>()
                    .map_err(|_| "input is not an HtmlInputElement".to_string())
            }) {
            Ok(input) => input,
            Err(err) => {
                log::error!("file picker setup failed: {err}");
                return;
            }
        };
        input.set_type("file");
        input.set_accept("image/png,image/jpeg");
        if let Err(err) = input.style().set_property("display", "none") {
            log::error!("hiding the file input failed: {err:?}");
        }

        let inbox = self.pending_image.clone();
        log::info!("DIAG file picker: change handler armed");
        let closure = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
            log::info!("DIAG file picker: change fired");
            let files = web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.get_element_by_id("e6_file_input"))
                .and_then(|element| element.dyn_into::<web_sys::HtmlInputElement>().ok())
                .and_then(|input| input.files())
                .and_then(|files| files.get(0));
            let Some(file) = files else { return };
            let inbox = inbox.clone();
            wasm_bindgen_futures::spawn_local(async move {
                // Blob::array_buffer()'s promise has been observed to never
                // settle in this setup; FileReader's event pipeline is the
                // reliable read path.
                let reader = web_sys::FileReader::new()
                    .map_err(|err| log::error!("FileReader creation failed: {err:?}"));
                let Ok(reader) = reader else { return };
                let reader = std::rc::Rc::new(reader);
                let buffer: js_sys::Promise = js_sys::Promise::new(
                    &mut |resolve: js_sys::Function, reject: js_sys::Function| {
                        let reader_for_load = reader.clone();
                        let reject_for_error = reject.clone();
                        let reject_for_load = reject.clone();
                        let onload =
                            wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
                                // FileReader::result() yields Result<JsValue, JsValue>;
                                // on `load` it is Ok(ArrayBuffer-as-JsValue).
                                match reader_for_load.result() {
                                    Ok(buffer) => {
                                        let _ =
                                            resolve.call1(&wasm_bindgen::JsValue::NULL, &buffer);
                                    }
                                    Err(err) => {
                                        let _ = reject_for_load.call0(&wasm_bindgen::JsValue::NULL);
                                        log::error!("FileReader result error: {err:?}");
                                    }
                                }
                            });
                        reader.set_onload(Some(onload.as_ref().unchecked_ref()));
                        // The closure owns a reader clone for as long as the
                        // browser holds the handler.
                        std::mem::forget(onload);
                        let onerror =
                            wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
                                let _ = reject_for_error.call0(&wasm_bindgen::JsValue::NULL);
                            });
                        reader.set_onerror(Some(onerror.as_ref().unchecked_ref()));
                        std::mem::forget(onerror);
                        if let Err(err) = reader.read_as_array_buffer(&file) {
                            log::error!("FileReader read failed: {err:?}");
                        }
                    },
                );
                match wasm_bindgen_futures::JsFuture::from(buffer).await {
                    Ok(buffer) => {
                        log::info!("file picker: bytes read");
                        let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                        if let Ok(mut slot) = inbox.lock() {
                            *slot = Some(bytes);
                        }
                    }
                    Err(err) => log::error!("reading the picked image failed: {err:?}"),
                }
            });
        });
        input.set_onchange(Some(closure.as_ref().unchecked_ref()));
        if let Some(body) = document.body()
            && let Err(err) = body.append_child(&input)
        {
            log::error!("attaching the file input failed: {err:?}");
        }
        // The element carries a stable id so the closure can re-find it without a cycle.
        input.set_id("e6_file_input");
        self.file_input = Some(input);
        self.file_closure = Some(closure);
    }

    #[cfg(target_arch = "wasm32")]
    fn restore_web(&mut self) {
        let inbox = self.pending_restore.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match storage::web::load_snapshot().await {
                Ok(Some(bytes)) => match BoardDoc::from_snapshot(&bytes) {
                    Ok(doc) => {
                        if let Ok(mut slot) = inbox.lock() {
                            *slot = Some(RestoreOutcome::Restored(doc));
                        }
                    }
                    Err(err) => log::error!("saved board is unusable ({err}); starting fresh"),
                },
                Ok(None) => {
                    log::info!("no saved board yet; showing demo");
                    if let Ok(mut slot) = inbox.lock() {
                        *slot = Some(RestoreOutcome::Empty);
                    }
                }
                Err(err) => log::error!("reading the saved board failed ({err}); starting fresh"),
            }
        });
    }

    /// Decode + place picked image bytes; the single entry point for both platforms.
    fn insert_image_bytes(&mut self, bytes: Vec<u8>) {
        match self.doc.insert_image(bytes) {
            Ok(id) => {
                self.doc.commit();
                self.dirty = true;
                self.selection.set([id]);
                self.tool = Tool::Select;
                // The file picker took focus; shortcuts live on the canvas.
                #[cfg(target_arch = "wasm32")]
                self.refocus_canvas();
            }
            Err(err) => log::error!("image insert failed: {err}"),
        }
    }

    /// Fresh installs open on a deterministic demo board, exactly once: the seeded flag
    /// persists in snapshots, so a board the user emptied stays empty.
    fn load_demo_board_if_unseeded(&mut self) {
        if self.doc.seeded() {
            return;
        }
        for element in welcome_board() {
            if let Err(err) = self.doc.insert_element(&ElementSpec::of(&element)) {
                log::error!("demo seed insert failed: {err}");
            }
        }
        if let Err(err) = self.doc.set_seeded() {
            log::error!("seed flag failed: {err}");
        }
        self.doc.commit();
        self.selection.clear();
        self.dirty = true;
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn restore_native(&mut self) {
        let path = storage::native::board_path();
        match storage::native::load(&path) {
            Ok(Some(bytes)) => match BoardDoc::from_snapshot(&bytes) {
                Ok(doc) => {
                    log::info!("restored board from {}", path.display());
                    self.replace_doc(doc);
                }
                Err(err) => log::error!(
                    "snapshot at {} is unusable ({err}); starting fresh",
                    path.display()
                ),
            },
            Ok(None) => log::info!("no saved board yet; autosaves to {}", path.display()),
            Err(err) => log::error!("reading {} failed ({err}); starting fresh", path.display()),
        }
    }

    /// Push a committed gesture's snapshot to local storage at most once concurrently.
    fn autosave(&mut self) {
        let snapshot = match self.doc.snapshot() {
            Ok(bytes) => bytes,
            Err(err) => {
                log::error!("snapshot export failed: {err}");
                return;
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = storage::native::board_path();
            if let Err(err) = storage::native::save(&path, &snapshot) {
                log::error!("autosave to {} failed: {err}", path.display());
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            if self.save_inflight.swap(true, Ordering::SeqCst) {
                return;
            }
            let inflight = self.save_inflight.clone();
            wasm_bindgen_futures::spawn_local(async move {
                if let Err(err) = storage::web::save_snapshot(&snapshot).await {
                    log::error!("OPFS autosave failed: {err}");
                }
                inflight.store(false, Ordering::SeqCst);
            });
        }
    }

    fn export_json(&mut self) {
        let dump = match self.doc.json_dump() {
            Ok(dump) => dump,
            Err(err) => {
                log::error!("JSON dump failed: {err}");
                return;
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = storage::native::json_path();
            match storage::native::save_text(&path, &dump) {
                Ok(()) => log::info!("board exported to {}", path.display()),
                Err(err) => log::error!("export to {} failed: {err}", path.display()),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            if let Err(err) = storage::web::download_text("e6paint-board.json", &dump) {
                log::error!("JSON download failed: {err}");
            }
        }
    }

    fn replace_doc(&mut self, doc: BoardDoc) {
        self.doc = doc;
        self.selection.clear();
        self.drag = None;
        self.draft = None;
        self.editing = None;
        self.fitted = false;
        self.dirty = true;
    }

    /// Fit `bounds` into `viewport` with a margin.
    fn camera_fit(bounds: Aabb, viewport: Size) -> Camera {
        let fit_zoom = (viewport.width / bounds.size().width)
            .min(viewport.height / bounds.size().height)
            * 0.9;
        let zoom = fit_zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let center = bounds.center();
        Camera {
            origin: Point::new(
                center.x - viewport.width / (2.0 * zoom),
                center.y - viewport.height / (2.0 * zoom),
            ),
            zoom,
        }
    }

    /// Frame the whole board on first sight of a real viewport.
    fn fit_camera(&mut self, viewport: Size) {
        let elements = self.doc.elements();
        let Some(first) = elements.first() else {
            return;
        };
        let mut bounds = first.bounds;
        for element in &elements {
            bounds = bounds.union(element.bounds);
        }
        self.camera = Self::camera_fit(bounds, viewport);
        self.fitted = true;
    }

    /// Present mode: animate the camera to frame `index`.
    fn present_at(&mut self, index: usize, viewport: Size) {
        let frames = self.doc.frames();
        if frames.is_empty() {
            return;
        }
        let index = index.min(frames.len() - 1);
        self.present = Some(index);
        self.camera_tween = Some(CameraTween {
            from: self.camera,
            to: Self::camera_fit(frames[index].bounds, viewport),
            started: Instant::now(),
        });
    }

    fn exit_present(&mut self) {
        self.present = None;
        self.camera_tween = None;
        self.fitted = false;
    }
    /// Advance the tween and handle present-mode navigation. Returns true while
    /// presenting (the caller then skips editing gestures).
    fn present_step(&mut self, ctx: &egui::Context, viewport: Size) -> bool {
        let Some(index) = self.present else {
            return false;
        };
        if let Some(tween) = self.camera_tween.as_mut() {
            self.camera = tween.camera_now();
            if tween.finished() {
                self.camera_tween = None;
            }
        }
        let (next, prev, exit) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowRight)
                    || input.key_pressed(egui::Key::Space)
                    || input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::ArrowLeft) || input.key_pressed(egui::Key::Backspace),
                input.key_pressed(egui::Key::Escape),
            )
        });
        if exit {
            self.exit_present();
            return false;
        }
        if next {
            self.present_at(index + 1, viewport);
        } else if prev {
            self.present_at(index.saturating_sub(1), viewport);
        }
        true
    }

    fn end_editing(&mut self) {
        if self.editing.take().is_some() {
            self.doc.commit();
            self.dirty = true;
        }
        self.editing_focused = false;
        // eframe's hidden IME input keeps DOM focus after the editor closes,
        // which silently kills every canvas-bound keyboard shortcut (keydown is
        // bound on the canvas). Hand focus back explicitly.
        #[cfg(target_arch = "wasm32")]
        self.refocus_canvas();
    }

    /// Hand DOM focus back to the canvas. Keyboard events are bound on the
    /// canvas, so anything that steals focus (the IME agent, the file input)
    /// must return it or every shortcut silently dies.
    #[cfg(target_arch = "wasm32")]
    fn refocus_canvas(&self) {
        use wasm_bindgen::JsCast;
        if let Some(canvas) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("e6paint_canvas"))
        {
            let _ = canvas.set_attribute("tabindex", "0");
            if let Ok(element) = canvas.dyn_into::<web_sys::HtmlElement>() {
                let _ = element.focus();
            }
        }
    }

    fn run_hud_command(&mut self, command: HudCommand) {
        let viewport = self.last_viewport;
        match command {
            HudCommand::None => {}
            HudCommand::Present => {
                self.end_editing();
                self.present_at(0, viewport);
            }
            HudCommand::Undo => {
                if let Err(err) = self.doc.undo() {
                    log::error!("undo failed: {err}");
                }
                self.selection.clear();
                self.dirty = true;
            }
            HudCommand::Redo => {
                if let Err(err) = self.doc.redo() {
                    log::error!("redo failed: {err}");
                }
                self.selection.clear();
                self.dirty = true;
            }
            HudCommand::Reset => {
                self.replace_doc(BoardDoc::new());
                if !self.room_first {
                    self.load_demo_board_if_unseeded();
                }
                self.dirty = true;
            }
            HudCommand::Align(mode) => self.apply_align(mode),
            HudCommand::ExportJson => self.export_json(),
            HudCommand::ExportSvg => self.export_svg(),
            HudCommand::ExportPng => self.export_png(),
            HudCommand::BringToFront => self.for_selected(|doc, id| doc.bring_to_front(id)),
            HudCommand::SendToBack => self.for_selected(|doc, id| doc.send_to_back(id)),
            HudCommand::Group => {
                let ids: Vec<ElementId> = self.selection.iter().collect();
                if ids.len() >= 2 {
                    if let Err(err) = self.doc.group(&ids) {
                        log::error!("group failed: {err}");
                    }
                    self.doc.commit();
                    self.dirty = true;
                }
            }
            HudCommand::Ungroup => {
                let ids: Vec<ElementId> = self.selection.iter().collect();
                if ids.is_empty() {
                    return;
                }
                if let Err(err) = self.doc.ungroup(&ids) {
                    log::error!("ungroup failed: {err}");
                }
                self.doc.commit();
                self.dirty = true;
            }
            HudCommand::ToggleLock => {
                let any_unlocked = self
                    .doc
                    .elements()
                    .iter()
                    .any(|e| self.selection.contains(e.id) && !e.locked);
                for id in self.selection.iter() {
                    if let Err(err) = self.doc.set_locked(id, any_unlocked) {
                        log::error!("lock failed: {err}");
                    }
                }
                self.doc.commit();
                self.dirty = true;
            }
            HudCommand::ShareEdit => {
                #[cfg(target_arch = "wasm32")]
                {
                    if let Some(session) = self.collab.as_ref() {
                        let location = web_sys::window()
                            .and_then(|window| window.location().href().ok())
                            .unwrap_or_default();
                        let base = match location.split_once('?') {
                            Some((base, _)) => base.to_owned(),
                            None => location,
                        };
                        // The write token grants full edit rights: keep it out of logs.
                        let link = format!(
                            "{base}?room={}&token={}",
                            session.room.room_id, session.room.token
                        );
                        storage::web::copy_to_clipboard(&link);
                        log::info!("edit link copied to the clipboard");
                    } else {
                        self.start_share();
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                log::info!("sharing is web-only");
            }
            HudCommand::ShareView => {
                #[cfg(target_arch = "wasm32")]
                {
                    if let Some(session) = self.collab.as_ref() {
                        let Some(read_token) = session.room.read_token.as_deref() else {
                            // Unreachable through the UI (the button is gated), but a
                            // stale command must still not pass silently.
                            log::warn!("no read token: only the room creator can share view links");
                            return;
                        };
                        let location = web_sys::window()
                            .and_then(|window| window.location().href().ok())
                            .unwrap_or_default();
                        let base = match location.split_once('?') {
                            Some((base, _)) => base.to_owned(),
                            None => location,
                        };
                        let link =
                            format!("{base}?room={}&token={read_token}", session.room.room_id);
                        storage::web::copy_to_clipboard(&link);
                        log::info!("view-only link copied to the clipboard");
                    } else {
                        self.start_share();
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                log::info!("sharing is web-only");
            }
            HudCommand::ToggleSketch => {
                let next = !self.doc.sketch();
                if let Err(err) = self.doc.set_sketch(next) {
                    log::error!("sketch toggle failed: {err}");
                }
                self.doc.commit();
                self.dirty = true;
            }
            HudCommand::InsertImage => {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let Some(path) = rfd::FileDialog::new()
                        .add_filter("Images", &["png", "jpg", "jpeg"])
                        .pick_file()
                    else {
                        return;
                    };
                    match std::fs::read(&path) {
                        Ok(bytes) => self.insert_image_bytes(bytes),
                        Err(err) => log::error!("reading {} failed: {err}", path.display()),
                    }
                }
                #[cfg(target_arch = "wasm32")]
                if let Some(input) = self.file_input.as_ref() {
                    input.click();
                }
            }
        }
    }

    fn handle_panel_action(&mut self, action: e6_ui::PanelAction, elements: &[Element]) {
        match action {
            e6_ui::PanelAction::None => {}
            e6_ui::PanelAction::Select(id) => {
                self.selection.set([id]);
                self.tool = Tool::Select;
            }
            e6_ui::PanelAction::ToggleHidden(id) => {
                let current = elements.iter().find(|e| e.id == id).map(|e| e.hidden);
                if let Some(hidden) = current {
                    if let Err(err) = self.doc.set_hidden(id, !hidden) {
                        log::error!("visibility toggle failed: {err}");
                    }
                    self.doc.commit();
                    self.dirty = true;
                }
            }
            e6_ui::PanelAction::Insert(entry) => self.insert_library_entry(entry),
        }
    }

    /// Stamp a library preset at the center of the current view.
    fn insert_library_entry(&mut self, entry: e6_ui::LibraryEntry) {
        use e6_ui::LibraryEntry;
        let center = Point::new(
            self.camera.origin.x + self.last_viewport.width / (2.0 * self.camera.zoom),
            self.camera.origin.y + self.last_viewport.height / (2.0 * self.camera.zoom),
        );
        let place = |offset: (f32, f32), size: (f32, f32)| {
            Aabb::from_min_size(
                Point::new(center.x + offset.0, center.y + offset.1),
                Size::new(size.0, size.1),
            )
        };
        let result = match entry {
            LibraryEntry::Sticky => self.doc.insert_element(
                &ElementSpec::simple(
                    ElementKind::Sticky,
                    place((-110.0, -110.0), (220.0, 220.0)),
                    STICKY_FILL,
                )
                .with_text("Note".into(), 24.0),
            ),
            LibraryEntry::Card => self.doc.insert_element(&ElementSpec::simple(
                ElementKind::Rect,
                place((-140.0, -80.0), (280.0, 160.0)),
                0xFF4C_8DFF,
            )),
            LibraryEntry::Heading => self.doc.insert_element(
                &ElementSpec::simple(
                    ElementKind::Text,
                    place((-360.0, -32.0), (720.0, 64.0)),
                    INK,
                )
                .with_text("Heading".into(), 48.0),
            ),
            LibraryEntry::Frame16x9 => self.doc.insert_element(
                &ElementSpec::simple(
                    ElementKind::Frame,
                    place((-640.0, -360.0), (1280.0, 720.0)),
                    0x0000_0000,
                )
                .outlined(0xFF7C_7CE8, 2.0)
                .with_text(format!("Section {}", self.doc.frames().len() + 1), 20.0),
            ),
            LibraryEntry::FlowTrio => {
                let inserted: Result<Vec<ElementId>, e6_doc::DocError> = (|| {
                    let a = self.doc.insert_element(
                        &ElementSpec::simple(
                            ElementKind::Rect,
                            place((-320.0, -40.0), (200.0, 80.0)),
                            0x0000_0000,
                        )
                        .outlined(INK, 2.0)
                        .with_text("A".into(), 28.0),
                    )?;
                    let b = self.doc.insert_element(
                        &ElementSpec::simple(
                            ElementKind::Rect,
                            place((120.0, -40.0), (200.0, 80.0)),
                            0x0000_0000,
                        )
                        .outlined(INK, 2.0)
                        .with_text("B".into(), 28.0),
                    )?;
                    let connector = self.doc.insert_element(
                        &ElementSpec::simple(
                            ElementKind::Connector,
                            Aabb::from_points(&[
                                Point::new(center.x - 120.0, center.y),
                                Point::new(center.x + 120.0, center.y),
                            ])
                            .ok_or(e6_doc::DocError::Corrupt("flow trio points".into()))?,
                            INK,
                        )
                        .outlined(INK, 2.0)
                        .with_points(vec![
                            Point::new(center.x - 120.0, center.y),
                            Point::new(center.x + 120.0, center.y),
                        ]),
                    )?;
                    Ok(vec![a, b, connector])
                })();
                match inserted {
                    Ok(ids) => {
                        if let Err(err) = self.doc.group(&ids) {
                            log::error!("library group failed: {err}");
                        }
                        self.doc.commit();
                        self.dirty = true;
                        self.selection.set(ids);
                        self.tool = Tool::Select;
                    }
                    Err(err) => log::error!("library insert failed: {err}"),
                }
                return;
            }
        };
        match result {
            Ok(id) => {
                self.doc.commit();
                self.dirty = true;
                self.selection.set([id]);
                self.tool = Tool::Select;
            }
            Err(err) => log::error!("library insert failed: {err}"),
        }
    }

    /// Parse + expand the code panel text and stamp the blocks as frames.
    fn expand_mol(&mut self) {
        let text = self.code_panel.text.clone();
        self.code_panel.errors.clear();
        let document = match e6_dsl::parse(&text) {
            Ok(document) => document,
            Err(errors) => {
                self.code_panel.errors = errors.iter().map(|err| err.to_string()).collect();
                return;
            }
        };
        let expansion = match e6_dsl::expand(&document, &e6_dsl::ExpandOptions::default()) {
            Ok(expansion) => expansion,
            Err(err) => {
                self.code_panel.errors.push(err.to_string());
                return;
            }
        };
        if expansion.blocks.is_empty() {
            self.code_panel
                .errors
                .push("nothing to expand: the document has no blocks".into());
            return;
        }

        // Re-apply: replace the previous generation (unlocked elements only).
        if self.mol_applied.is_some()
            && let Err(err) = self.doc.remove_by_provenance("mol:")
        {
            self.code_panel
                .errors
                .push(format!("re-apply removal failed: {err}"));
            return;
        }
        self.mol_applied = Some(text.clone());

        let center = self.viewport_center_world();
        let mut cursor_x = center.x;
        let center_y = center.y;
        let mut placed_any = false;
        let mut all_bounds: Option<Aabb> = None;

        for (block_index, block) in expansion.blocks.iter().enumerate() {
            let mut block_bounds: Option<Aabb> = None;
            let extend = |bounds: Aabb, acc: &mut Option<Aabb>| {
                *acc = Some(match acc {
                    Some(existing) => existing.union(bounds),
                    None => bounds,
                });
            };
            for node in &block.nodes {
                extend(
                    Aabb::from_min_size(node.position, node.size),
                    &mut block_bounds,
                );
            }
            for edge in &block.edges {
                if let Some(edge_bounds) = Aabb::from_points(&edge.points) {
                    extend(edge_bounds, &mut block_bounds);
                }
            }
            let Some(local) = block_bounds else { continue };
            let target_min = Point::new(
                cursor_x - local.size().width * 0.5,
                center_y - local.size().height * 0.5,
            );
            let offset = Point::new(target_min.x - local.min.x, target_min.y - local.min.y);
            cursor_x = target_min.x + local.size().width + 120.0;

            let frame_bounds = Aabb::from_min_size(
                Point::new(target_min.x - 40.0, target_min.y - 70.0),
                Size::new(local.size().width + 80.0, local.size().height + 100.0),
            );
            let block_prefix = format!("mol:{block_index}:");
            let frame = self.doc.insert_element(
                &ElementSpec::simple(ElementKind::Frame, frame_bounds, 0x0000_0000)
                    .outlined(0xFF7C_7CE8, 2.0)
                    .with_text(block.title.clone(), 20.0)
                    .with_provenance(format!("{block_prefix}frame")),
            );
            if let Err(err) = frame {
                self.code_panel
                    .errors
                    .push(format!("frame insert failed: {err}"));
                continue;
            }

            const SLICE_PALETTE: [u32; 6] = [
                0xFF5B_8DEF,
                0xFFED_9A6C,
                0xFF7B_C47F,
                0xFFE4_647A,
                0xFF9B_7BDE,
                0xFF4C_BCD4,
            ];
            let mut block_ids: Vec<ElementId> = Vec::new();
            for (node_index, node) in block.nodes.iter().enumerate() {
                let node_bounds = Aabb::from_min_size(
                    Point::new(node.position.x + offset.x, node.position.y + offset.y),
                    node.size,
                );
                let mut points: Vec<Point> = node
                    .points
                    .iter()
                    .map(|point| Point::new(point.x + offset.x, point.y + offset.y))
                    .collect();
                let (kind, fill) = match block.kind {
                    e6_dsl::BlockKind::Mind => (ElementKind::Text, INK),
                    e6_dsl::BlockKind::Pie if !points.is_empty() => (
                        ElementKind::Sector,
                        SLICE_PALETTE[node_index % SLICE_PALETTE.len()],
                    ),
                    e6_dsl::BlockKind::Git => (ElementKind::Ellipse, INK),
                    _ => (ElementKind::Rect, 0x0000_0000),
                };
                let mut spec = ElementSpec::simple(kind, node_bounds, fill)
                    .with_text(node.label.clone(), 20.0)
                    .with_provenance(format!("{block_prefix}node:{}", node.id));
                if kind == ElementKind::Rect {
                    spec = spec.outlined(INK, 2.0);
                }
                if kind == ElementKind::Sector {
                    spec = spec.with_points(std::mem::take(&mut points));
                }
                match self.doc.insert_element(&spec) {
                    Ok(id) => block_ids.push(id),
                    Err(err) => self
                        .code_panel
                        .errors
                        .push(format!("node insert failed: {err}")),
                }
            }
            for (edge_index, edge) in block.edges.iter().enumerate() {
                let points = [
                    Point::new(edge.points[0].x + offset.x, edge.points[0].y + offset.y),
                    Point::new(edge.points[1].x + offset.x, edge.points[1].y + offset.y),
                ];
                let Some(edge_bounds) = Aabb::from_points(&points) else {
                    continue;
                };
                let provenance = format!("{block_prefix}edge:{edge_index}");
                match self.doc.insert_element(
                    &ElementSpec::simple(ElementKind::Connector, edge_bounds, INK)
                        .outlined(INK, 2.0)
                        .with_points(points.to_vec())
                        .with_provenance(provenance),
                ) {
                    Ok(_) => {}
                    Err(err) => self
                        .code_panel
                        .errors
                        .push(format!("edge insert failed: {err}")),
                }
                if let Some(label) = &edge.label {
                    let mid = Point::new(
                        (points[0].x + points[1].x) * 0.5 - 40.0,
                        (points[0].y + points[1].y) * 0.5 - 24.0,
                    );
                    let label_prov = format!("{block_prefix}edge-label:{edge_index}");
                    if let Err(err) = self.doc.insert_element(
                        &ElementSpec::simple(
                            ElementKind::Text,
                            Aabb::from_min_size(mid, Size::new(120.0, 24.0)),
                            INK,
                        )
                        .with_text(label.clone(), 16.0)
                        .with_provenance(label_prov),
                    ) {
                        log::error!("edge label insert failed: {err}");
                    }
                }
            }
            placed_any = true;
            let frame_id = frame.ok();
            if let Some(frame_id) = frame_id {
                block_ids.push(frame_id);
            }
            for id in &block_ids {
                if let Err(err) = self.doc.reattach_to_frame(*id) {
                    log::error!("frame reattach failed for {id:?}: {err}");
                }
            }
            extend(frame_bounds, &mut all_bounds);
        }

        if placed_any {
            self.doc.commit();
            self.dirty = true;
            self.selection.clear();
            if let Some(bounds) = all_bounds {
                self.camera = Self::camera_fit(bounds, self.last_viewport);
            }
        } else {
            self.code_panel
                .errors
                .push("expansion produced no placeable content".into());
        }
    }

    /// Convert imported Mermaid text into mol in the editor, logging warnings.
    fn import_mermaid(&mut self) {
        let text = self.code_panel.text.clone();
        self.code_panel.errors.clear();
        match e6_dsl::import_mermaid(&text) {
            Ok(conversion) => {
                for warning in &conversion.warnings {
                    log::info!("mermaid import: {warning}");
                }
                self.code_panel.text = conversion.mol;
            }
            Err(err) => self.code_panel.errors.push(err),
        }
    }

    /// Web-only: kick off room creation + socket join.
    #[cfg(target_arch = "wasm32")]
    fn start_share(&mut self) {
        self.collab = None;
        self.notify("creating room...");
        self.join_flow = Some(crate::collab_ws::JoinFlow::start(
            self.relay_http_base.as_str(),
        ));
    }

    /// Build the collab session once the join flow produced a transport.
    #[cfg(target_arch = "wasm32")]
    fn build_session(&mut self, transport: crate::collab_ws::Transport) {
        let peer = format!("p{}", js_sys::Date::now() as u64);
        // Stable, cheap tint from the peer string (palette spread, not security).
        let tint_byte = 60 + (peer.bytes().map(u32::from).sum::<u32>() % 160);
        self.collab = Some(CollabSession {
            identity: ClientIdentity {
                peer: peer.clone(),
                name: format!("guest-{}", &peer[1..6]),
                color: 0xFF00_0000 | 0x00_8888 | (tint_byte << 16),
            },
            room: self
                .join_flow
                .as_ref()
                .and_then(|flow| flow.take_room())
                .unwrap_or(crate::collab_ws::RoomConnection {
                    room_id: String::new(),
                    token: String::new(),
                    read_token: None,
                }),
            transport: Some(transport),
            relay_http_base: self.relay_http_base.clone(),
            outbox: Outbox::new(),
            presence_throttle: SendThrottle::default(),
            backoff: Backoff::default(),
            reconnect_at: None,
            read_only: false,
            last_ping_sent: None,
            last_pong_at: None,
            remote_cursors: Vec::new(),
        });
    }

    /// Reopen the socket to the same room after a drop. The room and token are reused —
    /// no new room is created — and the server greeting merges the current snapshot.
    #[cfg(target_arch = "wasm32")]
    fn start_reconnect(&mut self) {
        let Some(session) = self.collab.as_mut() else {
            return;
        };
        let url = crate::collab_ws::to_ws_url(&session.relay_http_base, &session.room);
        match crate::collab_ws::Transport::connect(&url, self.ws_events.clone()) {
            Ok(transport) => {
                session.transport = Some(transport);
                session.backoff.reset();
                session.reconnect_at = None;
                // Announce our (possibly divergent) history; redundant batches merge
                // idempotently on every peer.
                self.dirty = true;
                log::info!("reconnected to room {}", session.room.room_id);
            }
            Err(err) => {
                let delay = session.backoff.fail();
                session.reconnect_at = Some(Instant::now() + delay);
                log::warn!("reconnect failed ({err}); retrying in {delay:?}");
            }
        }
    }

    /// Push local edits to the room (full-history batch, throttled by callers).
    /// Returns false only when a send/export failed — the caller keeps `dirty` set
    /// so the next frame retries. A closed or missing transport returns true:
    /// reconnects always resend full history, so nothing can be lost.
    #[cfg(target_arch = "wasm32")]
    fn flush_sync(&mut self) -> bool {
        let Some(session) = self.collab.as_mut().filter(|session| {
            !session.read_only && session.transport.as_ref().is_some_and(|t| t.is_open())
        }) else {
            return true;
        };
        match self.doc.full_updates() {
            Ok(batch) => {
                session.outbox.push(batch);
                if let Some(frame) = session.outbox.take() {
                    let frame = frame::wrap(frame::TAG_BATCH, &frame);
                    let Some(transport) = session.transport.as_ref() else {
                        return true;
                    };
                    if let Err(err) = transport.send(frame) {
                        log::error!("sync push failed: {err}");
                        self.notify("sync failed; retrying...");
                        return false;
                    }
                }
                true
            }
            Err(err) => {
                log::error!("update export failed: {err}");
                false
            }
        }
    }

    /// Surface a transient message in the HUD (the console is not enough — a user
    /// clicking Share must not watch a silent button).
    #[cfg(target_arch = "wasm32")]
    fn notify(&mut self, message: impl Into<String>) {
        self.hud.notice = Some(message.into());
        self.hud.notice_until = Some(Instant::now() + Duration::from_secs(8));
    }

    /// Apply one inbound socket event.
    #[cfg(target_arch = "wasm32")]
    fn handle_ws_event(&mut self, event: crate::collab_ws::WsEvent) {
        match event {
            crate::collab_ws::WsEvent::Failed(message) => {
                self.notify(format!("share failed: {message}"));
            }
            crate::collab_ws::WsEvent::Open => {}
            crate::collab_ws::WsEvent::Frame(bytes) => {
                let Some((tag, payload)) = frame::unwrap(&bytes) else {
                    return;
                };
                match tag {
                    frame::TAG_PONG => {
                        let session = self.collab.as_mut();
                        if let Some(session) = session {
                            session.last_pong_at = Some(Instant::now());
                        }
                    }
                    frame::TAG_WELCOME => {
                        let read_only = serde_json::from_slice::<serde_json::Value>(payload)
                            .ok()
                            .and_then(|value| value["read_only"].as_bool())
                            .unwrap_or(false);
                        let session = self.collab.as_mut();
                        if let Some(session) = session {
                            session.read_only = read_only;
                        }
                        if read_only {
                            log::info!("view-only session: edits stay on this device");
                            self.notify("view-only session: edits stay on this device");
                        }
                    }
                    frame::TAG_SNAPSHOT | frame::TAG_BATCH => {
                        if let Err(err) = self.doc.apply_remote(payload) {
                            log::error!("remote update rejected: {err}");
                        } else {
                            log::info!("remote board update applied ({} bytes)", payload.len());
                            self.fitted = false;
                        }
                    }
                    frame::TAG_PRESENCE => match e6_collab::presence_from_json(payload) {
                        Ok(update) => {
                            let peer = update.identity.peer;
                            let name = update.identity.name;
                            let at = update.cursor;
                            let color = update.identity.color;
                            let session = self.collab.as_mut();
                            let Some(session) = session else { return };
                            if session.identity.peer == peer {
                                return;
                            }
                            let cursors = &mut session.remote_cursors;
                            if let Some(slot) =
                                cursors.iter_mut().find(|cursor| cursor.peer == peer)
                            {
                                slot.at = at;
                                slot.name = name;
                            } else {
                                cursors.push(RemoteCursor {
                                    peer,
                                    name,
                                    color,
                                    at,
                                });
                            }
                        }
                        Err(err) => log::error!("{err}"),
                    },
                    _ => {}
                }
            }
            crate::collab_ws::WsEvent::Closed => {
                let (room_id, delay) = {
                    let Some(session) = self.collab.as_mut() else {
                        return;
                    };
                    let delay = session.backoff.fail();
                    session.reconnect_at = Some(Instant::now() + delay);
                    session.transport = None;
                    (session.room.room_id.clone(), delay)
                };
                log::warn!("room {room_id} socket closed; reconnecting in {delay:?}");
            }
        }
    }

    /// Center of the canvas in world space.
    /// Render the board to SVG and hand it to the platform (download / file).
    fn export_svg(&mut self) {
        let elements = self.doc.elements();
        let svg = e6_render::export_svg(&elements, &self.doc.title());
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = storage::native::svg_path();
            match storage::native::save_text(&path, &svg) {
                Ok(()) => log::info!("board exported to {}", path.display()),
                Err(err) => log::error!("SVG export to {} failed: {err}", path.display()),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            log::info!(
                "svg export: {} elements, {} bytes",
                elements.len(),
                svg.len()
            );
            if let Err(err) = storage::web::download_text("e6paint-board.svg", &svg) {
                log::error!("SVG download failed: {err}");
            }
        }
    }

    /// Render the board to PNG and hand it to the platform (download / file).
    fn export_png(&mut self) {
        let elements = self.doc.elements();
        let started = Instant::now();
        match e6_render::export_png(&elements, &self.doc.title(), 2) {
            Ok(bytes) => {
                log::info!(
                    "png export: {} elements to {} bytes in {:.0} ms",
                    elements.len(),
                    bytes.len(),
                    started.elapsed().as_millis()
                );
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let mut path = storage::native::json_path();
                    path.set_extension("png");
                    if let Err(err) = storage::native::save(&path, &bytes) {
                        log::error!("PNG export to {} failed: {err}", path.display());
                    } else {
                        log::info!("board exported to {}", path.display());
                    }
                }
                #[cfg(target_arch = "wasm32")]
                if let Err(err) =
                    storage::web::download_bytes("e6paint-board.png", "image/png", &bytes)
                {
                    log::error!("PNG download failed: {err}");
                }
            }
            Err(err) => self
                .code_panel
                .errors
                .push(format!("PNG export failed: {err}")),
        }
    }

    fn viewport_center_world(&self) -> Point {
        Point::new(
            self.camera.origin.x + self.last_viewport.width / (2.0 * self.camera.zoom),
            self.camera.origin.y + self.last_viewport.height / (2.0 * self.camera.zoom),
        )
    }

    fn apply_align(&mut self, mode: e6_editor::align::AlignMode) {
        let ids: Vec<ElementId> = self.selection.iter().collect();
        let elements = self.doc.elements();
        let items: Vec<(ElementId, Aabb)> = elements
            .iter()
            .filter(|e| ids.contains(&e.id))
            .map(|e| (e.id, e.bounds))
            .collect();
        let deltas = e6_editor::align::compute_deltas(&items, mode);
        if deltas.is_empty() {
            return;
        }
        for (id, delta) in deltas {
            if let Err(err) = self.doc.translate(id, delta.x, delta.y) {
                log::error!("align failed: {err}");
            }
        }
        self.doc.commit();
        self.dirty = true;
    }

    fn for_selected(&mut self, op: impl Fn(&BoardDoc, ElementId) -> Result<(), e6_doc::DocError>) {
        let ids: Vec<ElementId> = self.selection.iter().collect();
        if ids.is_empty() {
            return;
        }
        for id in &ids {
            if let Err(err) = op(&self.doc, *id) {
                log::error!("z-order op failed: {err}");
            }
        }
        self.doc.commit();
        self.dirty = true;
    }

    /// A key press from this frame's event stream with the modifiers captured on
    /// that same event. The shared `input.modifiers` state is unreliable on web —
    /// egui clears it between the DOM keydown and the frame read when the text
    /// agent's focus changes — so every modifier shortcut reads per-event state.
    fn key_pressed_with_modifiers(ctx: &egui::Context, key: egui::Key) -> Option<egui::Modifiers> {
        ctx.input(|input| {
            input.events.iter().rev().find_map(|event| match event {
                egui::Event::Key {
                    key: event_key,
                    pressed: true,
                    modifiers,
                    ..
                } if *event_key == key => Some(*modifiers),
                _ => None,
            })
        })
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let z = Self::key_pressed_with_modifiers(ctx, egui::Key::Z);
        let y = Self::key_pressed_with_modifiers(ctx, egui::Key::Y);
        let e = Self::key_pressed_with_modifiers(ctx, egui::Key::E);
        let enter = Self::key_pressed_with_modifiers(ctx, egui::Key::Enter);
        let delete = ctx.input(|input| {
            input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
        });
        let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));

        if escape {
            // While editing, the inline editor owns Escape (it applies its buffer
            // and closes in draw_text_editor, so queued keystrokes survive).
            if self.editing.is_some() {
                return;
            }
            if self.draft.take().is_some() || self.marquee.take().is_some() {
                // draft or marquee discarded
            } else {
                self.selection.clear();
            }
            return;
        }

        // Everything below acts on the board. A focused text field (the inline
        // editor or the mol code panel) owns the keyboard: its Delete, cmd/ctrl+Z/Y
        // and digits must edit text, never the board.
        let keyboard_busy = ctx.egui_wants_keyboard_input();

        // cmd/ctrl+Enter expands the mol panel — the standard "render the source"
        // gesture for diagram tools. It works even while typing in the panel, so
        // this check sits above the keyboard_busy guard; canvas text editing is
        // left via Escape first anyway.
        if let Some(modifiers) = enter
            && (modifiers.command || modifiers.ctrl)
        {
            self.expand_mol();
            return;
        }

        if keyboard_busy {
            return;
        }

        if delete && self.editing.is_none() {
            let ids: Vec<ElementId> = self.selection.iter().collect();
            if ids.is_empty() {
                return;
            }
            for id in &ids {
                if let Err(err) = self.doc.remove_element(*id) {
                    log::error!("delete failed: {err}");
                }
            }
            self.doc.commit();
            self.dirty = true;
            self.selection.clear();
            return;
        }

        // cmd/ctrl+E exports SVG; add shift for PNG. Exports must not fire while
        // a text field owns the keyboard.
        if let Some(modifiers) = e
            && (modifiers.command || modifiers.ctrl)
        {
            let command = if modifiers.shift {
                HudCommand::ExportPng
            } else {
                HudCommand::ExportSvg
            };
            self.run_hud_command(command);
            return;
        }

        let undo_modifiers = z
            .or(y)
            .filter(|modifiers| modifiers.command || modifiers.ctrl);
        if let Some(modifiers) = undo_modifiers {
            let outcome = if modifiers.shift {
                self.doc.redo()
            } else {
                self.doc.undo()
            };
            if let Err(err) = outcome {
                log::error!("undo/redo failed: {err}");
            }
            self.selection.clear();
            self.dirty = true;
        }

        // Digits 1–9 pick tools in tool-bar order (the bar shows the digit on hover).
        // keyboard_busy is already excluded above; the editing guard keeps the
        // shortcut inert even if the editor ever loses focus mid-edit. Reads
        // per-event state (see key_pressed_with_modifiers) — the global queue is
        // cleared by text-agent focus churn on room-first pages.
        if self.editing.is_none() {
            const DIGIT_KEYS: [egui::Key; 9] = [
                egui::Key::Num1,
                egui::Key::Num2,
                egui::Key::Num3,
                egui::Key::Num4,
                egui::Key::Num5,
                egui::Key::Num6,
                egui::Key::Num7,
                egui::Key::Num8,
                egui::Key::Num9,
            ];
            for (index, key) in DIGIT_KEYS.iter().enumerate() {
                if Self::key_pressed_with_modifiers(ctx, *key).is_some()
                    && let Some(tool) = e6_ui::tool_for_digit(index as u8 + 1)
                {
                    self.apply_tool(tool);
                }
            }
        }
    }

    /// Switch tools the way a tool-bar click does: leave text editing, drop any
    /// in-flight draft and marquee.
    fn apply_tool(&mut self, tool: Tool) {
        #[cfg(target_arch = "wasm32")]
        self.end_editing();
        self.draft = None;
        self.marquee = None;
        self.tool = tool;
    }

    /// Wheel pans; pinch or cmd/ctrl+wheel zooms at the pointer (the Excalidraw/Miro
    /// convention); primary-drag pans when the tool owns panning and no gesture is active.
    fn handle_camera_input(&mut self, response: &egui::Response, pixels_per_point: f32) {
        #[cfg(target_arch = "wasm32")]
        if response.hovered()
            && let Some(pos) = response.hover_pos()
            && let Some(world) = Some(self.camera.screen_to_world(canvas_pos(
                response,
                pos,
                pixels_per_point,
            )))
            && self.collab.is_some()
        {
            self.broadcast_presence(world, Instant::now());
        }

        let (scroll, zoom_delta, zoom_modifier) = response.ctx.input(|input| {
            (
                input.smooth_scroll_delta,
                input.zoom_delta(),
                input.modifiers.command || input.modifiers.ctrl,
            )
        });
        let pointer = response
            .hover_pos()
            .map(|pos| canvas_pos(response, pos, pixels_per_point));

        let wheel_zoom = zoom_delta != 1.0 || (zoom_modifier && scroll.y != 0.0);
        if wheel_zoom {
            let mut factor = zoom_delta;
            if zoom_modifier && scroll.y != 0.0 {
                factor *= (scroll.y * 0.01).exp();
            }
            if let Some(at) = pointer {
                self.camera.zoom_at(at, factor);
            }
        } else {
            self.camera.pan_by(Point::new(
                scroll.x * pixels_per_point,
                scroll.y * pixels_per_point,
            ));
        }

        let pans_on_drag = self.tool == Tool::Pan || self.space_held(response);
        if pans_on_drag && self.draft.is_none() && response.dragged() {
            let drag = response.drag_delta();
            self.camera.pan_by(Point::new(
                drag.x * pixels_per_point,
                drag.y * pixels_per_point,
            ));
        }
    }

    /// Web-only: push this client's cursor presence if the send window is open.
    #[cfg(target_arch = "wasm32")]
    fn broadcast_presence(&mut self, world: Point, now: Instant) {
        let Some(session) = self.collab.as_mut() else {
            return;
        };
        if !session
            .transport
            .as_ref()
            .is_some_and(|transport| transport.is_open())
            || !session.presence_throttle.ready(now)
        {
            return;
        }
        let update = e6_collab::PresenceUpdate {
            identity: session.identity.clone(),
            cursor: world,
        };
        let frame = frame::wrap(frame::TAG_PRESENCE, presence_json(&update).as_bytes());
        if let Some(transport) = session.transport.as_ref()
            && let Err(err) = transport.send(frame)
        {
            log::error!("presence send failed: {err}");
        }
    }

    /// Select gestures are suppressed while space is held: space+drag pans (the
    /// Miro convention), regardless of the active tool.
    fn space_held(&self, response: &egui::Response) -> bool {
        response.ctx.input(|input| input.key_down(egui::Key::Space))
    }

    fn world_at(&self, response: &egui::Response, pixels_per_point: f32) -> Option<Point> {
        response
            .interact_pointer_pos()
            .map(|pos| canvas_pos(response, pos, pixels_per_point))
            .map(|screen| self.camera.screen_to_world(screen))
    }

    /// Dispatch the active tool's gesture. `select` moves elements; creation tools build
    /// a draft and commit it as one element.
    fn handle_canvas_gesture(&mut self, response: &egui::Response, pixels_per_point: f32) {
        // Space+drag pans (handle_camera_input); placement/selection gestures yield.
        if self.space_held(response) {
            response.ctx.set_cursor_icon(egui::CursorIcon::Grab);
            return;
        }
        match self.tool {
            Tool::Select => self.select_gesture(response, pixels_per_point),
            Tool::Pan => {}
            Tool::Pen => self.pen_gesture(response, pixels_per_point),
            Tool::Rect | Tool::Ellipse => self.shape_gesture(response, pixels_per_point),
            Tool::Connector => self.connector_gesture(response, pixels_per_point),
            Tool::Text | Tool::Sticky | Tool::Frame => {
                self.place_gesture(response, pixels_per_point)
            }
            Tool::Image => {}
        }
    }

    fn select_gesture(&mut self, response: &egui::Response, pixels_per_point: f32) {
        let shift = response.ctx.input(|input| input.modifiers.shift);
        if self.space_held(response) {
            // Space+drag belongs to panning (handled in handle_camera_input).
            return;
        }
        // Double-click on a text-bearing element reopens its inline editor.
        if response.double_clicked()
            && let Some(world) = self.world_at(response, pixels_per_point)
            && let Some(element) = self.topmost_at(world)
            && matches!(element.kind, ElementKind::Text | ElementKind::Sticky)
        {
            self.selection.set([element.id]);
            self.editing = Some(element.id);
            return;
        }
        // Miro convention: double-click empty canvas drops a sticky and opens it.
        if response.double_clicked()
            && let Some(world) = self.world_at(response, pixels_per_point)
            && self.topmost_at(world).is_none()
        {
            self.place_sticky_at(world);
            return;
        }
        if response.drag_started()
            && let Some(press_world) = self.world_at(response, pixels_per_point)
        {
            match self.topmost_at(press_world) {
                Some(element) => {
                    if shift {
                        self.selection.toggle(element.id);
                    } else {
                        if !self.selection.contains(element.id) {
                            let mut members = match element.kind {
                                ElementKind::Frame => self.doc.frame_members(element.id),
                                _ => self.doc.group_members(element.id),
                            };
                            members.push(element.id);
                            self.selection.set(members);
                        }
                        self.drag = Some(DragState {
                            last_world: press_world,
                        });
                    }
                }
                None => {
                    self.selection.clear();
                    self.marquee = Some(MarqueeState {
                        start: press_world,
                        end: press_world,
                    });
                }
            }
        }

        if response.dragged()
            && let Some(drag) = self.drag.as_ref()
            && let Some(current) = self.world_at(response, pixels_per_point)
        {
            let dx = current.x - drag.last_world.x;
            let dy = current.y - drag.last_world.y;
            let locked: Vec<ElementId> = self
                .doc
                .elements()
                .iter()
                .filter(|e| e.locked && self.selection.contains(e.id))
                .map(|e| e.id)
                .collect();
            for id in self.selection.iter() {
                if locked.contains(&id) {
                    continue;
                }
                if let Err(err) = self.doc.translate(id, dx, dy) {
                    log::error!("move failed: {err}");
                }
            }
            if let Some(drag) = self.drag.as_mut() {
                drag.last_world = current;
            }
        }

        if response.drag_stopped() {
            if let Some(marquee) = self.marquee.take() {
                let region = marquee.bounds();
                let clicked_without_drag = region.size().width * self.camera.zoom < CLICK_SLOP_PX
                    && region.size().height * self.camera.zoom < CLICK_SLOP_PX;
                if !clicked_without_drag {
                    let hits: Vec<ElementId> = self
                        .doc
                        .elements()
                        .into_iter()
                        .filter(|e| !e.hidden && e.bounds.intersects(region))
                        .map(|e| e.id)
                        .collect();
                    self.selection.set(hits);
                }
            } else if self.drag.take().is_some() {
                // Dropping re-evaluates frame membership; commit keeps it in the same
                // undo step as the move.
                for id in self.selection.iter() {
                    if let Err(err) = self.doc.reattach_to_frame(id) {
                        log::error!("frame reattach failed for {id:?}: {err}");
                    }
                }
                // One release = one commit = one undo step, even if nothing moved.
                self.doc.commit();
                self.dirty = true;
            }
        }
    }

    fn pen_gesture(&mut self, response: &egui::Response, pixels_per_point: f32) {
        if response.drag_started() {
            if let Some(world) = self.world_at(response, pixels_per_point) {
                self.draft = Some(Draft::Pen {
                    points: vec![world],
                    last_screen: world,
                });
            }
            return;
        }
        if response.dragged() {
            let Some(world) = self.world_at(response, pixels_per_point) else {
                return;
            };
            if let Some(Draft::Pen {
                points,
                last_screen,
            }) = self.draft.as_mut()
            {
                let moved =
                    ((world.x - last_screen.x).hypot(world.y - last_screen.y)) * self.camera.zoom;
                if moved > PEN_SAMPLE_PX {
                    points.push(world);
                    *last_screen = world;
                }
            }
            return;
        }
        if response.drag_stopped()
            && let Some(Draft::Pen { points, .. }) = self.draft.take()
            && points.len() >= 2
            && let Some(bounds) = Aabb::from_points(&points)
        {
            let spec = ElementSpec::simple(ElementKind::Stroke, bounds, INK)
                .outlined(INK, 2.5)
                .with_points(points);
            match self.doc.insert_element(&spec) {
                Ok(_) => {
                    self.doc.commit();
                    self.dirty = true;
                }
                Err(err) => log::error!("stroke insert failed: {err}"),
            }
        }
    }

    fn shape_gesture(&mut self, response: &egui::Response, pixels_per_point: f32) {
        let kind = match self.tool {
            Tool::Rect => ElementKind::Rect,
            Tool::Ellipse => ElementKind::Ellipse,
            _ => return,
        };
        if response.drag_started()
            && let Some(world) = self.world_at(response, pixels_per_point)
        {
            self.draft = Some(Draft::Shape {
                kind,
                start: world,
                end: world,
            });
            return;
        }
        if response.dragged()
            && let Some(world) = self.world_at(response, pixels_per_point)
            && let Some(Draft::Shape { end, .. }) = self.draft.as_mut()
        {
            *end = world;
            return;
        }
        if response.drag_stopped()
            && let Some(Draft::Shape { kind, start, end }) = self.draft.take()
            && (start.x - end.x).hypot(start.y - end.y) * self.camera.zoom > CLICK_SLOP_PX
        {
            self.finish_creation(Draft::Shape { kind, start, end }.to_spec());
        }
    }

    fn connector_gesture(&mut self, response: &egui::Response, pixels_per_point: f32) {
        if response.drag_started()
            && let Some(world) = self.world_at(response, pixels_per_point)
        {
            self.draft = Some(Draft::Connector {
                start: world,
                end: world,
            });
            return;
        }
        if response.dragged()
            && let Some(world) = self.world_at(response, pixels_per_point)
            && let Some(Draft::Connector { end, .. }) = self.draft.as_mut()
        {
            *end = world;
            return;
        }
        if response.drag_stopped()
            && let Some(Draft::Connector { start, end }) = self.draft.take()
            && (start.x - end.x).hypot(start.y - end.y) * self.camera.zoom > CLICK_SLOP_PX
        {
            self.finish_creation(Draft::Connector { start, end }.to_spec());
        }
    }

    /// Text and sticky tools place on click, then hand over to the text editor.
    fn place_gesture(&mut self, response: &egui::Response, pixels_per_point: f32) {
        if !response.clicked() {
            return;
        }
        let Some(at) = self.world_at(response, pixels_per_point) else {
            return;
        };
        let (kind, size, fill, font) = match self.tool {
            Tool::Text => (ElementKind::Text, TEXT_PLACE_SIZE, INK, 28.0),
            Tool::Frame => (ElementKind::Frame, FRAME_SIZE, 0x0000_0000, 20.0),
            _ => (ElementKind::Sticky, STICKY_SIZE, STICKY_FILL, 24.0),
        };
        let short_side = match kind {
            ElementKind::Sticky => size,
            ElementKind::Frame => size * 0.75,
            _ => 48.0,
        };
        let name = if kind == ElementKind::Frame {
            format!("Frame {}", self.doc.frames().len() + 1)
        } else {
            String::new()
        };
        let mut spec = ElementSpec::simple(
            kind,
            Aabb::from_min_size(at, Size::new(size, short_side)),
            fill,
        )
        .with_text(name, font);
        if kind == ElementKind::Frame {
            spec = spec.outlined(0xFF7C_7CE8, 2.0);
        }
        match self.doc.insert_element(&spec) {
            Ok(id) => {
                self.doc.commit();
                self.dirty = true;
                self.selection.set([id]);
                self.editing = Some(id);
                self.editing_focused = false;
                self.tool = Tool::Select;
            }
            Err(err) => log::error!("placement failed: {err}"),
        }
    }

    /// Miro convention: double-click empty canvas drops a sticky and opens it.
    fn place_sticky_at(&mut self, world: Point) {
        let spec = ElementSpec::simple(
            ElementKind::Sticky,
            Aabb::from_min_size(world, Size::new(STICKY_SIZE, STICKY_SIZE)),
            STICKY_FILL,
        );
        match self.doc.insert_element(&spec) {
            Ok(id) => {
                self.doc.commit();
                self.dirty = true;
                self.selection.set([id]);
                self.editing = Some(id);
                self.editing_focused = false;
                self.tool = Tool::Select;
            }
            Err(err) => log::error!("sticky placement failed: {err}"),
        }
    }

    fn finish_creation(&mut self, spec: ElementSpec) {
        match self.doc.insert_element(&spec) {
            Ok(id) => {
                self.doc.commit();
                self.dirty = true;
                self.selection.set([id]);
                self.tool = Tool::Select;
            }
            Err(err) => log::error!("insert failed: {err}"),
        }
    }

    /// Topmost unlocked, visible element under a world point (paint-order reverse
    /// scan). Locked elements stay marquee-selectable so they can be unlocked again;
    /// hidden elements are unreachable from canvas entirely.
    fn topmost_at(&self, world: Point) -> Option<Element> {
        self.doc
            .elements()
            .into_iter()
            .rev()
            .find(|e| !e.locked && !e.hidden && e.bounds.contains(world))
    }

    /// The inline text editor for text/sticky elements, anchored over the canvas.
    fn draw_text_editor(&mut self, ctx: &egui::Context, pixels_per_point: f32) {
        let Some(id) = self.editing else { return };
        let elements = self.doc.elements();
        let Some(element) = elements.iter().find(|e| e.id == id) else {
            self.editing = None;
            self.editing_focused = false;
            return;
        };
        let screen = self.camera.world_to_screen(element.bounds.min);
        let zoom = self.camera.zoom;
        let mut text = element.text.clone();
        let width = element.bounds.size().width * zoom / pixels_per_point;
        let font_px = (element.font_size * zoom / pixels_per_point).max(10.0);
        drop(elements);
        let mut changed = false;
        let mut submitted = false;
        egui::Area::new(egui::Id::unique(("e6_text_editor", id.raw())))
            .fixed_pos(egui::Pos2::new(
                screen.x / pixels_per_point,
                screen.y / pixels_per_point,
            ))
            .show(ctx, |ui| {
                let response = egui::TextEdit::multiline(&mut text)
                    .desired_width(width)
                    .font(egui::FontId::proportional(font_px))
                    .show(ui);
                changed = response.response.changed();
                submitted =
                    response.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if !self.editing_focused {
                    response.response.request_focus();
                    self.editing_focused = true;
                }
            });
        if changed {
            if let Err(err) = self.doc.set_text(id, &text) {
                log::error!("text edit apply failed: {err}");
            }
            self.dirty = true;
        }
        // The editor owns Escape: applying the buffer before closing, so no
        // keystroke queued in the same frame is lost (handle_shortcuts defers
        // Escape to here while editing is open).
        let escape_now = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        if submitted || escape_now {
            self.end_editing();
        }
    }

    fn paint(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        pixels_per_point: f32,
        elements: &[Element],
    ) {
        let mut elements = elements.to_vec();
        if let Some(draft) = self.draft.as_ref() {
            let mut preview = Element::new(
                ElementId::from_raw(0),
                ElementKind::Rect,
                Aabb::from_min_size(Point::ZERO, Size::new(1.0, 1.0)),
                INK,
            );
            let spec = draft.to_spec();
            preview.kind = spec.kind;
            preview.bounds = spec.bounds;
            preview.stroke_color = spec.stroke_color;
            preview.stroke_width = spec.stroke_width;
            preview.points = spec.points.clone();
            elements.push(preview);
        }

        let viewport = Size::new(
            rect.width() * pixels_per_point,
            rect.height() * pixels_per_point,
        );
        let mut scene = Scene::new();
        let stats = build_display_list(
            &mut scene,
            &self.camera,
            viewport,
            &elements,
            self.doc.sketch(),
        );
        self.hud.zoom = self.camera.zoom;
        self.hud.visible_elements = stats.visible;

        let job = FrameJob {
            scene,
            width: viewport.width.max(1.0) as u32,
            height: viewport.height.max(1.0) as u32,
        };
        match self.frame_job.lock() {
            Ok(mut slot) => *slot = Some(job),
            Err(poisoned) => log::error!("frame job mutex poisoned: {poisoned}"),
        }

        let callback = eframe::egui_wgpu::Callback::new_paint_callback(
            rect,
            VelloCallback {
                gfx: self.gfx.clone(),
                job: self.frame_job.clone(),
            },
        );
        ui.painter().add(callback);
        draw_canvas_text(ui, &self.camera, &elements, pixels_per_point);

        // Remote presence cursors (collab sessions, web only).
        #[cfg(target_arch = "wasm32")]
        if let Some(session) = self.collab.as_ref() {
            for cursor in &session.remote_cursors {
                let screen = self.camera.world_to_screen(cursor.at);
                let pos = egui::Pos2::new(screen.x / pixels_per_point, screen.y / pixels_per_point);
                let tint = egui::Color32::from_rgba_unmultiplied(
                    ((cursor.color >> 16) & 0xFF) as u8,
                    ((cursor.color >> 8) & 0xFF) as u8,
                    (cursor.color & 0xFF) as u8,
                    255,
                );
                ui.painter().circle_filled(pos, 5.0, tint);
                ui.painter().text(
                    pos + egui::vec2(8.0, -2.0),
                    egui::Align2::LEFT_TOP,
                    cursor.name.as_str(),
                    egui::FontId::proportional(12.0),
                    tint,
                );
            }
        }

        // Hover affordance: the element the cursor would grab lights up before
        // the click (Select tool only, not while dragging or space-panning).
        if self.tool == Tool::Select
            && self.drag.is_none()
            && !ui.input(|i| i.key_down(egui::Key::Space))
            && let Some(pos) = ui.input(|i| i.pointer.hover_pos())
        {
            let world = canvas_pos_in(rect, pos, pixels_per_point);
            if let Some(element) = self.topmost_at(world)
                && !self.selection.contains(element.id)
            {
                let min = self.camera.world_to_screen(element.bounds.min);
                let max = self.camera.world_to_screen(element.bounds.max);
                let hover_rect = egui::Rect::from_two_pos(
                    egui::Pos2::new(min.x / pixels_per_point, min.y / pixels_per_point),
                    egui::Pos2::new(max.x / pixels_per_point, max.y / pixels_per_point),
                );
                ui.painter().rect_stroke(
                    hover_rect,
                    0.0,
                    egui::Stroke::new(
                        1.5 * pixels_per_point,
                        egui::Color32::from_rgb(90, 145, 255),
                    ),
                    egui::StrokeKind::Middle,
                );
            }
        }

        // Marquee selection rectangle, drawn as canvas chrome above the scene.
        if let Some(marquee) = self.marquee.as_ref() {
            let a = self.camera.world_to_screen(marquee.start);
            let b = self.camera.world_to_screen(marquee.end);
            let rect = egui::Rect::from_two_pos(
                egui::Pos2::new(a.x / pixels_per_point, a.y / pixels_per_point),
                egui::Pos2::new(b.x / pixels_per_point, b.y / pixels_per_point),
            );
            ui.painter().rect_filled(
                rect,
                0.0,
                egui::Color32::from_rgba_unmultiplied(90, 145, 255, 26),
            );
            ui.painter().rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 145, 255)),
                egui::StrokeKind::Middle,
            );
        }

        self.draw_selection_overlay(ui, rect, pixels_per_point);

        self.draw_minimap(ui, rect, pixels_per_point);
        self.draw_zoom_widget(ui, rect);
        draw_status_bar(ui, &self.hud);
    }

    /// Selection chrome: Figma-blue outlines on every selected element, drag
    /// handles on single selection. Drawn above the scene so a click always
    /// shows what it hit.
    fn draw_selection_overlay(
        &mut self,
        ui: &mut egui::Ui,
        canvas_rect: egui::Rect,
        pixels_per_point: f32,
    ) {
        if self.selection.is_empty() {
            return;
        }
        let painter = ui.painter_at(canvas_rect);
        let stroke = egui::Stroke::new(
            1.5 * pixels_per_point,
            egui::Color32::from_rgb(13, 153, 255),
        );
        let count = self.selection.len();
        let elements = self.doc.elements();
        for id in self.selection.iter() {
            let Some(element) = elements.iter().find(|e| e.id == id) else {
                continue;
            };
            let min = self.camera.world_to_screen(element.bounds.min);
            let max = self.camera.world_to_screen(element.bounds.max);
            let rect = egui::Rect::from_two_pos(
                egui::Pos2::new(min.x / pixels_per_point, min.y / pixels_per_point),
                egui::Pos2::new(max.x / pixels_per_point, max.y / pixels_per_point),
            );
            painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Middle);
            if count == 1 {
                for corner in [
                    rect.left_top(),
                    rect.right_top(),
                    rect.left_bottom(),
                    rect.right_bottom(),
                ] {
                    let handle = egui::Rect::from_center_size(corner, egui::Vec2::splat(8.0));
                    painter.rect_filled(handle, 2.0, egui::Color32::WHITE);
                    painter.rect_stroke(handle, 2.0, stroke, egui::StrokeKind::Middle);
                }
            }
        }
    }

    /// Bottom-left zoom widget, Miro-style: zoom out, percentage, zoom in, fit.
    /// Widget actions zoom around the canvas center.
    fn draw_zoom_widget(&mut self, ui: &mut egui::Ui, canvas_rect: egui::Rect) {
        self.last_canvas_rect = Some(canvas_rect);
        egui::Area::new(egui::Id::unique("e6_zoom_widget"))
            .anchor(egui::Align2::LEFT_BOTTOM, [12.0, -12.0])
            .show(ui, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(0xFF, 0xFF, 0xFF))
                    .stroke(egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgb(224, 224, 230),
                    ))
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .corner_radius(6.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let center_screen = canvas_rect.center();
                            let world_center = self
                                .camera
                                .screen_to_world(Point::new(center_screen.x, center_screen.y));
                            if ui.button("−").clicked() {
                                self.camera.zoom_at(world_center, 1.0 / 1.2);
                            }
                            let zoom_pct = format!("{}%", (self.camera.zoom * 100.0) as u32);
                            if ui
                                .add(egui::Button::new(egui::RichText::new(zoom_pct).small()))
                                .clicked()
                            {
                                // Click the percentage to reset to 100%.
                                self.camera.zoom_at(world_center, 1.0 / self.camera.zoom);
                            }
                            if ui.button("+").clicked() {
                                self.camera.zoom_at(world_center, 1.2);
                            }
                            ui.separator();
                            if ui
                                .add(egui::Button::new(egui::RichText::new("Fit").small()))
                                .clicked()
                            {
                                let viewport = Size::new(
                                    canvas_rect.width() * ui.ctx().pixels_per_point(),
                                    canvas_rect.height() * ui.ctx().pixels_per_point(),
                                );
                                self.fit_camera(viewport);
                                self.fitted = true;
                            }
                        });
                    });
            });
    }
}

/// Bottom-right minimap implementation lives with the app below.
impl E6App {
    /// Minimap: element bounds scaled into a fixed box, viewport outlined, click/drag
    /// to move the camera. Dense boards draw a capped sample.
    fn draw_minimap(&mut self, ui: &mut egui::Ui, available: egui::Rect, pixels_per_point: f32) {
        let elements = self.doc.elements();
        if elements.is_empty() {
            return;
        }
        let (box_w, box_h) = (170.0_f32, 120.0_f32);
        let map_rect = egui::Rect::from_min_size(
            egui::Pos2::new(
                available.right() - box_w - 10.0,
                available.bottom() - box_h - 10.0,
            ),
            egui::Vec2::new(box_w, box_h),
        );

        let mut bounds = elements[0].bounds;
        for element in &elements {
            bounds = bounds.union(element.bounds);
        }
        let scale =
            (box_w / bounds.size().width.max(1.0)).min(box_h / bounds.size().height.max(1.0));
        // Center the fitted content inside the box, Miro-style.
        let offset = egui::Vec2::new(
            (box_w - bounds.size().width * scale) / 2.0,
            (box_h - bounds.size().height * scale) / 2.0,
        );
        let to_map = |point: Point| {
            egui::Pos2::new(
                map_rect.left() + offset.x + (point.x - bounds.min.x) * scale,
                map_rect.top() + offset.y + (point.y - bounds.min.y) * scale,
            )
        };

        ui.painter().rect_filled(
            map_rect,
            4.0,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 235),
        );
        let step = elements.len().div_ceil(2000);
        for (index, element) in elements.iter().enumerate() {
            if index % step != 0 {
                continue;
            }
            let a = to_map(element.bounds.min);
            let b = to_map(element.bounds.max);
            ui.painter().rect_filled(
                egui::Rect::from_two_pos(a, b),
                0.0,
                egui::Color32::from_rgba_unmultiplied(120, 120, 150, 150),
            );
        }
        let viewport_world = self.camera.viewport_aabb(Size::new(
            available.width() * pixels_per_point,
            available.height() * pixels_per_point,
        ));
        let viewport_rect =
            egui::Rect::from_two_pos(to_map(viewport_world.min), to_map(viewport_world.max));
        ui.painter().rect_stroke(
            viewport_rect,
            0.0,
            egui::Stroke::new(1.0, egui::Color32::WHITE),
            egui::StrokeKind::Middle,
        );

        let response = ui.allocate_rect(map_rect, egui::Sense::click_and_drag());
        if (response.clicked() || response.dragged())
            && let Some(pos) = response.interact_pointer_pos()
        {
            let world = Point::new(
                bounds.min.x + (pos.x - map_rect.left() - offset.x) / scale,
                bounds.min.y + (pos.y - map_rect.top() - offset.y) / scale,
            );
            let viewport_size = Size::new(
                available.width() * pixels_per_point,
                available.height() * pixels_per_point,
            );
            self.camera.origin = Point::new(
                world.x - viewport_size.width / (2.0 * self.camera.zoom),
                world.y - viewport_size.height / (2.0 * self.camera.zoom),
            );
        }
    }
}

impl eframe::App for E6App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Miro-light chrome: panels, menus, and widgets render light everywhere.
        static LIGHT: std::sync::Once = std::sync::Once::new();
        LIGHT.call_once(|| ui.ctx().set_theme(egui::Theme::Light));
        let now = Instant::now();
        if let Some(last) = self.last_frame {
            let delta_ms = now.duration_since(last).as_secs_f32() * 1000.0;
            self.hud.frame_ms = if self.hud.frame_ms == 0.0 {
                delta_ms
            } else {
                self.hud.frame_ms + FRAME_EMA_WEIGHT * (delta_ms - self.hud.frame_ms)
            };
        }
        self.last_frame = Some(now);
        if !self.painted_once {
            self.painted_once = true;
            log::info!("first frame painted");
        }

        let pixels_per_point = ui.ctx().pixels_per_point();
        let available = ui.available_rect_before_wrap();
        let viewport = Size::new(
            available.width() * pixels_per_point,
            available.height() * pixels_per_point,
        );
        self.last_viewport = viewport;
        let elements = self.doc.elements();
        self.hud.total_elements = elements.len();
        self.hud.selected_elements = self.selection.len();

        // Present mode owns the input: advance the tween, navigate, skip editing.
        if self.present_step(ui.ctx(), viewport) {
            if let Some(index) = self.present {
                let frames = self.doc.frames();
                let label = format!(
                    "Frame {} / {} \u{b7} \u{2192} next \u{b7} \u{2190} back \u{b7} Esc exit",
                    index + 1,
                    frames.len()
                );
                ui.painter().text(
                    egui::Pos2::new(available.center().x, 18.0),
                    egui::Align2::CENTER_TOP,
                    label,
                    egui::FontId::proportional(14.0),
                    egui::Color32::from_rgba_unmultiplied(235, 235, 245, 220),
                );
            }
            self.paint(
                ui,
                ui.available_rect_before_wrap(),
                pixels_per_point,
                &elements,
            );
            ui.ctx().request_repaint();
            return;
        }

        // Drain the async web restore, the picked image, then flush a pending autosave.
        #[cfg(target_arch = "wasm32")]
        {
            let restored = self
                .pending_restore
                .lock()
                .ok()
                .and_then(|mut slot| slot.take());
            match restored {
                Some(RestoreOutcome::Restored(doc)) => {
                    log::info!("restored board from OPFS");
                    self.replace_doc(doc);
                }
                // An empty store is what seeds the demo; a read error starts
                // empty (the fresh-user demo path only fires once per store).
                Some(RestoreOutcome::Empty) => self.load_demo_board_if_unseeded(),
                None => {}
            }
            let picked = self
                .pending_image
                .lock()
                .ok()
                .and_then(|mut slot| slot.take());
            if let Some(bytes) = picked {
                self.insert_image_bytes(bytes);
            }
        }
        // Collaboration drain: join flow completion, socket events, sync + presence.
        #[cfg(target_arch = "wasm32")]
        if let Some(flow) = self.join_flow.as_ref()
            && let Some(transport) = flow.take_transport()
        {
            log::info!("collab session joined");
            self.notify("connected to the room");
            self.build_session(transport);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let events = self
                .join_flow
                .as_ref()
                .map(|flow| flow.drain_events())
                .unwrap_or_default();
            for event in events {
                self.handle_ws_event(event);
            }
            // Liveness heartbeat: ping on an interval; a ping without a pong
            // inside the timeout means the transport is dead (no OS-level error
            // fires for half-open sockets), so force a reconnect.
            if let Some(session) = self.collab.as_mut()
                && session.transport.as_ref().is_some_and(|t| t.is_open())
            {
                let now = Instant::now();
                const PING_INTERVAL: Duration = Duration::from_secs(5);
                const PONG_TIMEOUT: Duration = Duration::from_secs(15);
                let ping_due = session
                    .last_ping_sent
                    .map(|sent| now.duration_since(sent) >= PING_INTERVAL)
                    .unwrap_or(true);
                if ping_due {
                    session.last_ping_sent = Some(now);
                    let frame = frame::wrap(frame::TAG_PING, &[]);
                    if let Some(transport) = session.transport.as_ref()
                        && let Err(err) = transport.send(frame)
                    {
                        log::error!("ping send failed: {err}");
                        let delay = session.backoff.fail();
                        session.transport = None;
                        session.reconnect_at = Some(now + delay);
                    }
                }
                let stale = e6_collab::pong_timed_out(
                    session.last_ping_sent,
                    session.last_pong_at,
                    now,
                    PONG_TIMEOUT,
                );
                if stale {
                    log::warn!("connection lost (no pong); reconnecting");
                    let delay = session.backoff.fail();
                    session.transport = None;
                    session.reconnect_at = Some(now + delay);
                }
            }

            // Auto-reconnect: when a dropped session's retry window elapses, reopen the
            // socket to the same room.
            let due = self
                .collab
                .as_ref()
                .map(|session| {
                    session.transport.is_none()
                        && session.reconnect_at.is_some_and(|at| Instant::now() >= at)
                })
                .unwrap_or(false);
            if due {
                self.start_reconnect();
            }
            // Persist + sync local changes: autosave (local sessions only -
            // room-first content belongs to the room), then the room batch.
            // `dirty` clears only after both succeeded; flush failures re-flag.
            if let Some(until) = self.hud.notice_until
                && Instant::now() >= until
            {
                self.hud.notice = None;
                self.hud.notice_until = None;
            }
            if self.dirty {
                if !self.room_first {
                    self.autosave();
                }
                let mut synced = true;
                if self.collab.as_ref().is_some_and(|s| {
                    s.transport
                        .as_ref()
                        .is_some_and(|transport| transport.is_open())
                }) {
                    synced = self.flush_sync();
                }
                if synced {
                    self.dirty = false;
                }
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.dirty {
            self.autosave();
            self.dirty = false;
        }

        if let Some(picked) = draw_tool_bar(ui, self.tool) {
            self.apply_tool(picked);
        }
        self.hud.title = self.doc.title();
        let availability = HudAvailability {
            can_undo: self.doc.can_undo(),
            can_redo: self.doc.can_redo(),
            has_selection: !self.selection.is_empty(),
            can_group: self.selection.len() >= 2,
            any_selected_locked: self
                .doc
                .elements()
                .iter()
                .any(|e| self.selection.contains(e.id) && e.locked),
            sketch: self.doc.sketch(),
            // The read token exists only on web sessions; native has no collab.
            #[cfg(target_arch = "wasm32")]
            can_share_view: self
                .collab
                .as_ref()
                .is_some_and(|session| session.room.read_token.is_some()),
            #[cfg(not(target_arch = "wasm32"))]
            can_share_view: false,
        };
        let command = draw_top_bar(ui, &self.hud, &availability);
        self.run_hud_command(command);
        self.handle_shortcuts(ui.ctx());

        // The right drawer consumes canvas space, so it draws before the canvas.
        let (drawer_tab, panel_action, code_action) = e6_ui::draw_right_drawer(
            ui,
            self.drawer_tab,
            &elements,
            &self.selection,
            &mut self.code_panel,
        );
        self.drawer_tab = drawer_tab;
        if let Some(action) = panel_action {
            self.handle_panel_action(action, &elements);
        }
        match code_action {
            e6_ui::CodeAction::None => {}
            e6_ui::CodeAction::Expand => self.expand_mol(),
            e6_ui::CodeAction::ImportMermaid => self.import_mermaid(),
        }

        let available = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(available, egui::Sense::click_and_drag());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Canvas, true, "e6paint board canvas")
        });
        // Tool-appropriate cursor (Miro convention); space+drag overrides to grab
        // inside handle_canvas_gesture.
        response.ctx.set_cursor_icon(match self.tool {
            Tool::Select => egui::CursorIcon::Default,
            Tool::Pan => egui::CursorIcon::Grab,
            Tool::Text => egui::CursorIcon::Text,
            Tool::Pen
            | Tool::Rect
            | Tool::Ellipse
            | Tool::Connector
            | Tool::Sticky
            | Tool::Frame => egui::CursorIcon::Crosshair,
            Tool::Image => egui::CursorIcon::Default,
        });
        let rect = response.rect;
        // A canvas press ends text editing (the click itself is consumed by the editor
        // area, which sits above the canvas).
        if response.drag_started() || response.clicked() {
            self.end_editing();
        }
        self.handle_camera_input(&response, pixels_per_point);
        self.handle_canvas_gesture(&response, pixels_per_point);
        if !self.fitted && rect.is_positive() {
            let viewport = Size::new(
                rect.width() * pixels_per_point,
                rect.height() * pixels_per_point,
            );
            self.fit_camera(viewport);
        }
        self.paint(ui, rect, pixels_per_point, &elements);
        self.draw_text_editor(ui.ctx(), pixels_per_point);

        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
mod canvas_pos_tests {
    use super::*;

    /// Regression: pointer math used absolute egui coordinates, displacing every
    /// selection by the panel footprint (the canvas rect starts inside panels).
    #[test]
    fn canvas_pos_is_rect_relative_physical() {
        let rect =
            egui::Rect::from_min_size(egui::Pos2::new(52.0, 0.0), egui::Vec2::new(800.0, 600.0));
        let world_screen = canvas_pos_in(rect, egui::Pos2::new(600.0, 300.0), 2.0);
        assert_eq!(world_screen, Point::new((600.0 - 52.0) * 2.0, 300.0 * 2.0));
    }
}
