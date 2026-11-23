// SPDX-License-Identifier: MIT
//! Theurgy client integration:
//! - Typed bidirectional payloads (`theurgy:book`, `theurgy:wand_mode`, `theurgy:filter_edit`)
//! - Hermetica book screen (`BookCanvas` over `DrawContext` + `ClientHost`)
//! - Server-side per-player Hermetica progress persistence
//! - Recipe viewer & Filter editor screen dispatch
//! - Key mappings, wand/in-world HUD layer, derivative jar item decorations

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use mod_api::client::hud_layout::PoseOp;
use mod_api::serde_json;
use mod_api::text::TextStyle;
use mod_api::{
    ClientHost, DrawContext, Flow, GuiDraw, GuiLayer, GuiLayerPlacement, Hand,
    IPayloadContext, Identifier, InputKeyEvent, InteractionKey,
    InteractionKeyMappingTriggeredEvent, ItemDecorationsEvent, ItemStack, KeyAction,
    KeyRegistration, MouseScrollingEvent, PayloadCodec, PayloadId, PayloadWork,
    RegisterGuiLayersEvent, Registrar, ScreenCharTypedPreEvent, ScreenKeyPressedPreEvent,
    ScreenMouseButtonPressedPreEvent, ScreenMouseDraggedPreEvent, ScreenMouseScrolledPreEvent,
    ScreenRenderPostEvent, ServerHost, SoundPlay, Text, ToClient, ToServer, VarInt,
};

use crate::{book, filter, filter_client, host, hud, items, recipe_client, selection};

/// Hermetica book data bundled with the package (`data/` is packed in `.hmod`).
const HERMETICA_JSON: &[u8] = include_bytes!("../data/theurgy/hermetica.json");

// ---------------------------------------------------------------------------
// Typed network payloads
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PayloadCodec)]
pub struct BookProgressPayload {
    pub json: String,
}

#[derive(Clone, Debug, PayloadCodec)]
pub struct WandModePayload {
    pub delta: VarInt,
}

/// A left click with the wand in its select-frequency mode: the frequency moves by `delta`.
#[derive(Clone, Debug, PayloadCodec)]
pub struct WandFrequencyPayload {
    pub delta: VarInt,
}

#[derive(Clone, Debug, PayloadCodec)]
pub struct FilterEditPayload {
    pub off_hand: bool,
    pub action: String,
    pub args: String,
}

/// The acting player's pending emitter selection: six numbers per outline box, one colour each.
#[derive(Clone, Debug, PayloadCodec)]
pub struct SelectionPayload {
    pub boxes: Vec<f64>,
    pub colors: Vec<i32>,
}

static SELECTION_TO_CLIENT: OnceLock<ToClient<SelectionPayload>> = OnceLock::new();

fn selection_outlines(payload: &SelectionPayload) -> Vec<([f64; 6], u32)> {
    payload
        .boxes
        .chunks_exact(6)
        .zip(&payload.colors)
        .map(|(b, color)| (std::array::from_fn(|i| b[i]), *color as u32 & 0x00FF_FFFF))
        .collect()
}

/// Server: tell `player_id`'s client which blocks to outline. In singleplayer there is no
/// remote session to send through, and the client half of this same process is set directly.
pub fn send_selection(
    host_ref: &mut dyn ServerHost,
    player_id: u128,
    outlines: Vec<([f64; 6], u32)>,
) {
    let payload = SelectionPayload {
        boxes: outlines.iter().flat_map(|(b, _)| *b).collect(),
        colors: outlines.iter().map(|(_, color)| *color as i32).collect(),
    };
    let sent = SELECTION_TO_CLIENT
        .get()
        .zip(host_ref.player(player_id))
        .is_some_and(|(token, player)| token.send(host_ref, player, &payload).is_ok());
    if !sent {
        selection::set_outlines(outlines);
    }
}

static BOOK_TO_SERVER: OnceLock<ToServer<BookProgressPayload>> = OnceLock::new();
static BOOK_TO_CLIENT: OnceLock<ToClient<BookProgressPayload>> = OnceLock::new();
static WAND_TO_SERVER: OnceLock<ToServer<WandModePayload>> = OnceLock::new();
static FREQUENCY_TO_SERVER: OnceLock<ToServer<WandFrequencyPayload>> = OnceLock::new();
static FILTER_TO_SERVER: OnceLock<ToServer<FilterEditPayload>> = OnceLock::new();

static BOOK_PROGRESS: Mutex<BTreeMap<u128, Vec<u8>>> = Mutex::new(BTreeMap::new());
static LOCAL_PLAYER_ID: Mutex<Option<u128>> = Mutex::new(None);

enum LocalAction {
    BookSave(Vec<u8>),
    WandMode(i32),
    WandFrequency(i32),
    HudRequest([i32; 3]),
    FilterEdit {
        hand: Hand,
        action: String,
        args: String,
    },
}

static PENDING_LOCAL_ACTIONS: Mutex<Vec<LocalAction>> = Mutex::new(Vec::new());

fn queue_local_action(action: LocalAction) {
    if let Ok(mut pending) = PENDING_LOCAL_ACTIONS.lock() {
        pending.push(action);
    }
}

/// Drain singleplayer client->integrated-server actions queued when no remote
/// payload session exists (`PayloadSendError::Disconnected`).
pub fn drain_local_actions(host_ref: &mut dyn ServerHost) {
    let actions: Vec<LocalAction> = PENDING_LOCAL_ACTIONS
        .lock()
        .ok()
        .map(|mut pending| std::mem::take(&mut *pending))
        .unwrap_or_default();
    if actions.is_empty() {
        return;
    }
    let player_id = host_ref
        .players()
        .acting_player()
        .and_then(|p| p.entity().uuid(host_ref))
        .or_else(|| LOCAL_PLAYER_ID.lock().ok().and_then(|g| *g));
    let Some(player_id) = player_id else {
        return;
    };
    for action in actions {
        match action {
            LocalAction::BookSave(bytes) => {
                if serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(&bytes)
                    .is_ok()
                    && let Ok(mut all) = BOOK_PROGRESS.lock()
                {
                    all.insert(player_id, bytes);
                }
            }
            LocalAction::WandMode(delta) => {
                items::change_wand_mode(host_ref, player_id, delta.clamp(-1, 1));
            }
            LocalAction::WandFrequency(delta) => {
                items::change_wand_frequency(host_ref, player_id, delta.clamp(-1, 1));
            }
            LocalAction::HudRequest(pos) => hud::answer(host_ref, player_id, pos),
            LocalAction::FilterEdit { hand, action, args } => {
                let _ = filter::apply_action(host_ref, player_id, hand, &action, &args);
            }
        }
    }
}

struct BookSaveWork(Vec<u8>);
impl PayloadWork for BookSaveWork {
    fn run(&self, cx: &mut dyn IPayloadContext) -> Result<(), String> {
        if let (Ok(player), Ok(host_ref)) = (cx.player(), cx.server_host())
            && let Some(uuid) = player.entity().uuid(host_ref)
            && let Ok(mut all) = BOOK_PROGRESS.lock()
        {
            all.insert(uuid, self.0.clone());
        }
        Ok(())
    }
}

struct WandModeWork(i32);
impl PayloadWork for WandModeWork {
    fn run(&self, cx: &mut dyn IPayloadContext) -> Result<(), String> {
        if let (Ok(player), Ok(host_ref)) = (cx.player(), cx.server_host())
            && let Some(uuid) = player.entity().uuid(host_ref)
        {
            items::change_wand_mode(host_ref, uuid, self.0);
        }
        Ok(())
    }
}

struct WandFrequencyWork(i32);
impl PayloadWork for WandFrequencyWork {
    fn run(&self, cx: &mut dyn IPayloadContext) -> Result<(), String> {
        if let (Ok(player), Ok(host_ref)) = (cx.player(), cx.server_host())
            && let Some(uuid) = player.entity().uuid(host_ref)
        {
            items::change_wand_frequency(host_ref, uuid, self.0);
        }
        Ok(())
    }
}

/// Singleplayer path of the in-world HUD's request (no remote session to send it through).
pub fn queue_hud_request(pos: [i32; 3]) {
    queue_local_action(LocalAction::HudRequest(pos));
}

struct FilterEditWork {
    hand: Hand,
    action: String,
    args: String,
}
impl PayloadWork for FilterEditWork {
    fn run(&self, cx: &mut dyn IPayloadContext) -> Result<(), String> {
        if let (Ok(player), Ok(host_ref)) = (cx.player(), cx.server_host())
            && let Some(uuid) = player.entity().uuid(host_ref)
        {
            let _ = filter::apply_action(host_ref, uuid, self.hand, &self.action, &self.args);
        }
        Ok(())
    }
}

pub fn register_payloads(r: &mut dyn Registrar) {
    let mut payloads = r.payload_handlers();
    let mut view = payloads.registrar("1").optional();

    let (book_c2s, book_s2c) = view.play_bidirectional_typed::<BookProgressPayload>(
        PayloadId("theurgy:book".into()),
        |value, cx| {
            let bytes = value.json.into_bytes();
            if serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(&bytes).is_ok()
            {
                let _ = cx.enqueue_work(Arc::new(BookSaveWork(bytes)));
            }
            Ok(())
        },
        |value, _cx| {
            book::set_progress(value.json.as_bytes());
            Ok(())
        },
    );
    let _ = BOOK_TO_SERVER.set(book_c2s);
    let _ = BOOK_TO_CLIENT.set(book_s2c);

    let selection_s2c = view.play_to_client_typed::<SelectionPayload>(
        PayloadId("theurgy:selection".into()),
        |value, _cx| {
            selection::set_outlines(selection_outlines(&value));
            Ok(())
        },
    );
    let _ = SELECTION_TO_CLIENT.set(selection_s2c);

    let wand_c2s = view.play_to_server_typed::<WandModePayload>(
        PayloadId("theurgy:wand_mode".into()),
        |value, cx| {
            let delta = value.delta.0.clamp(-1, 1);
            let _ = cx.enqueue_work(Arc::new(WandModeWork(delta)));
            Ok(())
        },
    );
    let _ = WAND_TO_SERVER.set(wand_c2s);

    let frequency_c2s = view.play_to_server_typed::<WandFrequencyPayload>(
        PayloadId("theurgy:wand_frequency".into()),
        |value, cx| {
            let delta = value.delta.0.clamp(-1, 1);
            let _ = cx.enqueue_work(Arc::new(WandFrequencyWork(delta)));
            Ok(())
        },
    );
    let _ = FREQUENCY_TO_SERVER.set(frequency_c2s);

    hud::register(&mut view);

    let filter_c2s = view.play_to_server_typed::<FilterEditPayload>(
        PayloadId("theurgy:filter_edit".into()),
        |value, cx| {
            let hand = if value.off_hand {
                Hand::Off
            } else {
                Hand::Main
            };
            let _ = cx.enqueue_work(Arc::new(FilterEditWork {
                hand,
                action: value.action,
                args: value.args,
            }));
            Ok(())
        },
    );
    let _ = FILTER_TO_SERVER.set(filter_c2s);
}

/// The keybinding category's translation key (`key.category.theurgy.keybindings` in the lang).
const KEY_CATEGORY: &str = "key.category.theurgy.keybindings";

pub fn register_keys(r: &mut dyn Registrar) {
    let mut mode = KeyRegistration::new("change_item_mode", "");
    mode.label = "key.theurgy.change_item_mode".to_owned();
    mode.category = KEY_CATEGORY.to_owned();
    r.register_key_mapping(mode);

    let mut fit = KeyRegistration::new("show_held_stack_fit_outline", "");
    fit.label = "key.theurgy.show_held_stack_fit_outline".to_owned();
    fit.category = KEY_CATEGORY.to_owned();
    r.register_key_mapping(fit);
}

pub fn register_gui_layers(event: &mut RegisterGuiLayersEvent<'_>) {
    event.layers.push(GuiLayer {
        id: hud::LAYER_ID.to_owned(),
        placement: GuiLayerPlacement::AboveAll,
    });
}

// ---------------------------------------------------------------------------
// Server-side Hermetica progress persistence
// ---------------------------------------------------------------------------

fn book_file(directory: &Path, player: u128) -> PathBuf {
    let h = format!("{player:032x}");
    directory.join(format!(
        "{}-{}-{}-{}-{}.theurgy_book.json",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}

pub fn book_load(player: u128, directory: &Path) {
    let bytes = std::fs::read(book_file(directory, player)).unwrap_or_else(|_| b"{}".to_vec());
    if let Ok(mut all) = BOOK_PROGRESS.lock() {
        all.insert(player, bytes);
    }
}

pub fn book_save(player: u128, directory: &Path) {
    let bytes = BOOK_PROGRESS
        .lock()
        .ok()
        .and_then(|all| all.get(&player).cloned());
    if let Some(bytes) = bytes {
        let _ = std::fs::write(book_file(directory, player), bytes);
    }
}

pub fn book_logged_in(host_ref: &mut dyn ServerHost, player_id: u128) {
    if let Ok(mut local) = LOCAL_PLAYER_ID.lock() {
        *local = Some(player_id);
    }
    let bytes = BOOK_PROGRESS
        .lock()
        .ok()
        .and_then(|all| all.get(&player_id).cloned())
        .unwrap_or_else(|| b"{}".to_vec());
    if let Some(token) = BOOK_TO_CLIENT.get()
        && let Some(player) = host_ref.player(player_id)
        && let Ok(json) = String::from_utf8(bytes.clone())
    {
        if token
            .send(host_ref, player, &BookProgressPayload { json })
            .is_err()
        {
            book::set_progress(&bytes);
        }
    } else {
        book::set_progress(&bytes);
    }
}

// ---------------------------------------------------------------------------
// Hermetica BookCanvas over DrawContext
// ---------------------------------------------------------------------------

enum BookOp {
    Sprite {
        name: String,
        src: [f32; 4],
        dst: [f32; 4],
        tint: u32,
    },
    Item {
        id: String,
        at: [f32; 2],
        size: f32,
    },
    Fill {
        rect: [f32; 4],
        argb: u32,
    },
    Text {
        text: Text,
        at: [f32; 2],
        shadow: bool,
        /// The glyphs' own scale about `at` (`scale_since`).
        scale: f32,
    },
    PushClip([f32; 4]),
    PopClip,
    Tooltip {
        lines: Vec<Text>,
        at: [f32; 2],
    },
}

struct BookCanvas<'a> {
    host: &'a dyn ClientHost,
    ops: Vec<BookOp>,
}

fn book_text(runs: &[book::Run]) -> Text {
    let mut out: Option<Text> = None;
    for run in runs {
        let mut style = TextStyle::default();
        style.color = Some(format!("#{:06x}", run.rgb & 0xFF_FFFF));
        style.bold = Some(run.bold);
        style.italic = Some(run.italic);
        if run.book_font {
            style.font = Some("theurgy:hermetica".to_owned());
        }
        let part = Text::literal(run.text.clone()).with_style(style);
        out = Some(match out {
            Some(prev) => prev.append(part),
            None => part,
        });
    }
    out.unwrap_or_else(|| Text::literal(String::new()))
}

/// One line of book text: at its own size, or under a pose scaled about its top left, so a page
/// shrunk to fit (`scale_since`) shrinks its glyphs with its line positions.
fn book_text_draws(text: Text, at: [f32; 2], shadow: bool, scale: f32) -> Vec<GuiDraw> {
    let line = |x: i32, y: i32| GuiDraw::Text {
        text: text.clone(),
        x,
        y,
        color: 0xFFFF_FFFF,
        shadow,
    };
    if (scale - 1.0).abs() < 1e-3 {
        return vec![line(at[0].round() as i32, at[1].round() as i32)];
    }
    vec![
        GuiDraw::Push(vec![
            PoseOp::Translate(at[0], at[1]),
            PoseOp::Scale(scale, scale),
        ]),
        line(0, 0),
        GuiDraw::Pop,
    ]
}

impl BookCanvas<'_> {
    fn flush(self, draw: &mut dyn DrawContext) {
        let host = self.host;
        for op in self.ops {
            match op {
                BookOp::Sprite {
                    name,
                    src,
                    dst,
                    tint,
                } => {
                    let _ = draw.sprite(&name, src, dst, Some(tint));
                }
                BookOp::Item { id, at, size } => {
                    if let Some(key) = host::item_key(host, &id) {
                        if !draw.item(key, at, size)
                            && let Some(fallback) = host::item_key(host, "theurgy:the_hermetica")
                        {
                            let _ = draw.item_model(fallback, &id, at, size);
                        }
                    } else if let Some(fallback) = host::item_key(host, "theurgy:the_hermetica") {
                        let _ = draw.item_model(fallback, &id, at, size);
                    }
                }
                BookOp::Fill { rect, argb } => draw.fill(rect, argb),
                BookOp::Text {
                    text,
                    at,
                    shadow,
                    scale,
                } => {
                    let _ = draw.gui_paint(&book_text_draws(text, at, shadow, scale));
                }
                BookOp::PushClip(rect) => {
                    let _ = draw.push_clip(rect);
                }
                BookOp::PopClip => {
                    let _ = draw.pop_clip();
                }
                BookOp::Tooltip { lines, at } => {
                    let _ = draw.gui_tooltip(&lines, at);
                }
            }
        }
    }
}

impl book::Canvas for BookCanvas<'_> {
    fn size(&self, name: &str) -> [f32; 2] {
        Identifier::parse(name)
            .ok()
            .and_then(|id| self.host.gui_sprite(&id))
            .map_or([0.0, 0.0], |r| [r.size[0] as f32, r.size[1] as f32])
    }

    fn sprite(&mut self, name: &str, src: [f32; 4], dst: [f32; 4], tint: u32) -> bool {
        self.ops.push(BookOp::Sprite {
            name: name.to_owned(),
            src,
            dst,
            tint,
        });
        true
    }

    fn item(&mut self, id: &str, at: [f32; 2], size: f32) -> bool {
        self.ops.push(BookOp::Item {
            id: id.to_owned(),
            at,
            size,
        });
        true
    }

    fn fill(&mut self, rect: [f32; 4], argb: u32) {
        self.ops.push(BookOp::Fill { rect, argb });
    }

    fn text(&mut self, runs: &[book::Run], at: [f32; 2], shadow: bool) {
        self.ops.push(BookOp::Text {
            text: book_text(runs),
            at,
            shadow,
            scale: 1.0,
        });
    }

    fn width(&self, runs: &[book::Run]) -> f32 {
        self.host
            .text_width(&book_text(runs))
            .unwrap_or_else(|_| runs.iter().map(|r| r.text.len() as i32 * 6).sum()) as f32
    }

    fn mark(&self) -> usize {
        self.ops.len()
    }

    fn clip_since(&mut self, mark: usize, rect: [f32; 4]) {
        if mark <= self.ops.len() {
            self.ops.insert(mark, BookOp::PushClip(rect));
            self.ops.push(BookOp::PopClip);
        }
    }

    fn scale_since(&mut self, mark: usize, origin: [f32; 2], scale: f32) {
        if mark >= self.ops.len() {
            return;
        }
        let scale_pt = |p: [f32; 2]| {
            [
                origin[0] + (p[0] - origin[0]) * scale,
                origin[1] + (p[1] - origin[1]) * scale,
            ]
        };
        for op in &mut self.ops[mark..] {
            match op {
                BookOp::Sprite { dst, .. } => {
                    let p = scale_pt([dst[0], dst[1]]);
                    *dst = [p[0], p[1], dst[2] * scale, dst[3] * scale];
                }
                BookOp::Item { at, size, .. } => {
                    *at = scale_pt(*at);
                    *size *= scale;
                }
                BookOp::Fill { rect, .. } => {
                    let p = scale_pt([rect[0], rect[1]]);
                    *rect = [p[0], p[1], rect[2] * scale, rect[3] * scale];
                }
                BookOp::Text {
                    at, scale: glyphs, ..
                } => {
                    *at = scale_pt(*at);
                    *glyphs *= scale;
                }
                _ => {}
            }
        }
    }

    fn tooltip(&mut self, lines: &[Vec<book::Run>], at: [f32; 2]) {
        self.ops.push(BookOp::Tooltip {
            lines: lines.iter().map(|l| book_text(l)).collect(),
            at,
        });
    }

    fn item_tooltip(&mut self, id: &str, at: [f32; 2]) {
        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
        let line = Text::translate(format!("item.{ns}.{path}"), Vec::new());
        self.ops.push(BookOp::Tooltip {
            lines: vec![line],
            at,
        });
    }

    fn time(&self) -> i64 {
        self.host.clock().map_or(0, |c| c.game_time)
    }

    fn shift(&self) -> bool {
        self.host.modifiers().shift()
    }
}

pub fn open_book(host_ref: &mut dyn ClientHost) -> bool {
    recipe_client::close();
    filter_client::close();
    if !book::open(HERMETICA_JSON, &|key| host_ref.translate(key)) {
        return false;
    }
    host_ref.mod_screen_open("theurgy");
    true
}

pub fn open_filter(host_ref: &mut dyn ClientHost, stack: &ItemStack, hand: Hand) -> bool {
    book::close();
    recipe_client::close();
    filter_client::open(host_ref, stack, hand)
}

// ---------------------------------------------------------------------------
// Screen & tick dispatch
// ---------------------------------------------------------------------------

pub fn screen_render_post(host_ref: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
    if event.screen.id != "mod" || host_ref.mod_screen() != Some("theurgy") {
        return;
    }
    if recipe_client::render(host_ref, &mut *event.draw, event.screen) {
        return;
    }
    if filter_client::render(host_ref, &mut *event.draw, event.screen) {
        return;
    }
    let [w, h] = event.draw.size();
    let mut canvas = BookCanvas {
        host: host_ref,
        ops: Vec::new(),
    };
    book::render(&mut canvas, w, h, event.screen.mouse);
    canvas.flush(&mut *event.draw);
}

pub fn client_tick_post(host_ref: &mut dyn ClientHost) {
    if book::take_turn_sound() {
        let _ = host_ref.play_local_sound(&SoundPlay::ui("minecraft:item.book.page_turn"), 0);
    }
    if let Some(bytes) = book::take_progress()
        && let Ok(json) = String::from_utf8(bytes.clone())
    {
        let sent = BOOK_TO_SERVER
            .get()
            .is_some_and(|token| token.send(host_ref, &BookProgressPayload { json }).is_ok());
        if !sent {
            queue_local_action(LocalAction::BookSave(bytes));
        }
    }
    for edit in filter_client::take_actions() {
        let sent = FILTER_TO_SERVER.get().is_some_and(|token| {
            token
                .send(
                    host_ref,
                    &FilterEditPayload {
                        off_hand: edit.hand == Hand::Off,
                        action: edit.action.clone(),
                        args: edit.args.clone(),
                    },
                )
                .is_ok()
        });
        if !sent {
            queue_local_action(LocalAction::FilterEdit {
                hand: edit.hand,
                action: edit.action,
                args: edit.args,
            });
        }
    }
    filter_client::tick(host_ref);
    hud::client_tick(host_ref);
}

fn book_screen(host_ref: &dyn ClientHost, screen_id: &str) -> bool {
    screen_id == "mod" && host_ref.mod_screen() == Some("theurgy")
}

pub fn screen_mouse_pressed(
    host_ref: &dyn ClientHost,
    event: &mut ScreenMouseButtonPressedPreEvent<'_>,
) -> Flow {
    if !book_screen(host_ref, event.screen.id) {
        return Flow::Continue;
    }
    if event.button == 0 && recipe_client::click(host_ref, event.screen, event.button) {
        return Flow::Cancel;
    }
    if event.button == 0 && filter_client::click(host_ref, event.screen, event.button) {
        return Flow::Cancel;
    }
    if book::click(
        event.screen.rect[2],
        event.screen.rect[3],
        event.screen.mouse,
        event.button,
        host_ref.modifiers().shift(),
    ) {
        return Flow::Cancel;
    }
    Flow::Continue
}

pub fn screen_mouse_dragged(
    host_ref: &dyn ClientHost,
    event: &mut ScreenMouseDraggedPreEvent<'_>,
) -> Flow {
    if book_screen(host_ref, event.screen.id) && book::drag(event.button, event.delta) {
        return Flow::Cancel;
    }
    Flow::Continue
}

pub fn screen_mouse_scrolled(
    host_ref: &dyn ClientHost,
    event: &mut ScreenMouseScrolledPreEvent<'_>,
) -> Flow {
    if !book_screen(host_ref, event.screen.id) {
        return Flow::Continue;
    }
    if recipe_client::scroll(host_ref, event.screen, event.scroll[1]) {
        return Flow::Cancel;
    }
    if filter_client::scroll(host_ref, event.screen, event.scroll[1]) {
        return Flow::Cancel;
    }
    if book::scroll(event.scroll[1]) {
        return Flow::Cancel;
    }
    Flow::Continue
}

pub fn screen_key_pressed(
    host_ref: &dyn ClientHost,
    event: &mut ScreenKeyPressedPreEvent<'_>,
) -> Flow {
    if !book_screen(host_ref, event.screen.id) {
        return Flow::Continue;
    }
    if filter_client::key_pressed(host_ref, event.screen, event.key) {
        return Flow::Cancel;
    }
    if book::key(event.key) {
        return Flow::Cancel;
    }
    Flow::Continue
}

pub fn screen_char_typed(
    host_ref: &dyn ClientHost,
    event: &mut ScreenCharTypedPreEvent<'_>,
) -> Flow {
    if !book_screen(host_ref, event.screen.id) {
        return Flow::Continue;
    }
    if filter_client::char_typed(host_ref, event.screen, event.c) {
        return Flow::Cancel;
    }
    if book::char_typed(event.c) {
        return Flow::Cancel;
    }
    Flow::Continue
}

// ---------------------------------------------------------------------------
// Input hooks: keys, scroll, interaction key (open book/filter)
// ---------------------------------------------------------------------------

pub fn local_hand_stack(host_ref: &dyn ClientHost, hand: Hand) -> Option<ItemStack> {
    let player = host_ref.local_player()?;
    host_ref.living(player.entity())?.hand_item(host_ref, hand)
}

pub fn interaction_key(
    host_ref: &mut dyn ClientHost,
    event: &mut InteractionKeyMappingTriggeredEvent,
) -> Flow {
    if event.cancelled || event.key != InteractionKey::Use {
        return Flow::Continue;
    }
    let Some(stack) = local_hand_stack(host_ref, event.hand) else {
        return Flow::Continue;
    };
    let id = stack.item().id(host_ref);
    if id == "theurgy:the_hermetica" && open_book(host_ref) {
        event.cancelled = true;
        return Flow::Cancel;
    }
    if matches!(id.as_str(), "theurgy:list_filter" | "theurgy:attribute_filter")
        && host_ref.raycast(6.0).is_none()
    {
        book::close();
        recipe_client::close();
        if filter_client::open(host_ref, &stack, event.hand) {
            event.cancelled = true;
            return Flow::Cancel;
        }
    }
    Flow::Continue
}

pub fn input_key(host_ref: &mut dyn ClientHost, event: &mut InputKeyEvent<'_>) {
    if event.action != KeyAction::Press || !host_ref.playing() {
        return;
    }
    if host_ref.consume_click("theurgy", "change_item_mode") {
        let sent = WAND_TO_SERVER
            .get()
            .is_some_and(|token| token.send(host_ref, &WandModePayload { delta: VarInt(1) }).is_ok());
        if !sent {
            queue_local_action(LocalAction::WandMode(1));
        }
    }
}

/// `SelectFrequencyMode.onLeftClickEmpty`: a left click at nothing lowers the wand's selected
/// frequency. The click never reaches the server on its own, so the client tells it.
pub fn left_click_empty(host_ref: &mut dyn ClientHost) {
    let selecting = local_hand_stack(host_ref, Hand::Main).is_some_and(|stack| {
        stack.item().id(host_ref) == items::MERCURIAL_WAND && items::wand_mode(host_ref, &stack) == 2
    });
    if !selecting {
        return;
    }
    let sent = FREQUENCY_TO_SERVER.get().is_some_and(|token| {
        token.send(host_ref, &WandFrequencyPayload { delta: VarInt(-1) }).is_ok()
    });
    if !sent {
        queue_local_action(LocalAction::WandFrequency(-1));
    }
}

pub fn mouse_scrolling(
    host_ref: &mut dyn ClientHost,
    event: &mut MouseScrollingEvent,
) -> Flow {
    let shift = host_ref.modifiers().shift()
        || host_ref.is_key_down("key.keyboard.left.shift")
        || host_ref.is_key_down("key.keyboard.right.shift");
    let delta = if event.accumulated[1] != 0 {
        event.accumulated[1].signum()
    } else if event.scroll[1] > 0.0 {
        1
    } else if event.scroll[1] < 0.0 {
        -1
    } else {
        0
    };
    if event.cancelled || !shift || delta == 0 {
        return Flow::Continue;
    }
    let Some(stack) = local_hand_stack(host_ref, Hand::Main) else {
        return Flow::Continue;
    };
    if stack.item().id(host_ref) != items::MERCURIAL_WAND {
        return Flow::Continue;
    }
    let sent = WAND_TO_SERVER
        .get()
        .is_some_and(|token| token.send(host_ref, &WandModePayload { delta: VarInt(delta) }).is_ok());
    if !sent {
        queue_local_action(LocalAction::WandMode(delta));
    }
    event.cancelled = true;
    Flow::Cancel
}

// ---------------------------------------------------------------------------
// Derivative jar item decorations & HUD layer
// ---------------------------------------------------------------------------

pub fn derivative_source(host_ref: &dyn ClientHost, stack: &ItemStack) -> Option<String> {
    if let Some(item) = items::component_string(host_ref, stack, "theurgy:source_item")
        && host::item_key(host_ref, &item).is_some()
    {
        return Some(item);
    }
    let tag = items::component_string(host_ref, stack, "theurgy:source_tag")?;
    let tag = tag.trim_start_matches('#');
    let id = Identifier::parse(tag).ok()?;
    let key = host_ref.tags().key(&id)?;
    let first = host_ref.tags().items(key)?.into_iter().next()?;
    Some(first.id(host_ref))
}

pub fn derivative_tier(host_ref: &dyn ClientHost, stack: &ItemStack, item_id: &str) -> Option<&'static str> {
    if item_id.starts_with("theurgy:alchemical_niter_") {
        return ["abundant", "common", "commond", "rare", "precious"]
            .into_iter()
            .find(|tier| item_id.ends_with(&format!("_{tier}")))
            .map(|tier| if tier == "commond" { "common" } else { tier });
    }
    ["abundant", "common", "rare", "precious"]
        .into_iter()
        .find(|tier| {
            host::in_item_tag(
                host_ref,
                stack.item(),
                &format!("theurgy:alchemical_sulfurs/{tier}"),
            )
        })
}

/// `AlchemicalDerivativeRenderer`'s shift view: in a slot, holding Shift shows the jar's source
/// item at full size over it. The jar itself, with its frame, label and small source item, is
/// the item's special model (`derivative.rs`) in every context.
pub fn item_decorations(host_ref: &dyn ClientHost, event: &mut ItemDecorationsEvent<'_>) {
    let item_id = event.stack.item().id(host_ref);
    if !item_id.starts_with("theurgy:alchemical_sulfur_")
        && !item_id.starts_with("theurgy:alchemical_niter_")
    {
        return;
    }
    if host_ref.modifiers().shift()
        && let Some(source_id) = derivative_source(host_ref, event.stack)
        && let Some(key) = host::item_key(host_ref, &source_id)
    {
        let _ = event.draw.item(key, [event.x, event.y], 16.0);
    }
}
