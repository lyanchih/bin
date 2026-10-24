mod config;
mod fetcher;
mod font;
mod http;
mod json;
mod render;

use config::{load_config, save_config, AppConfig, Orientation, ScaleSize};
use fetcher::{fetch_all, QuotaSummary};
use render::{Color, Framebuffer};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionBtn {
    Orientation,
    Resize,
    Settings,
    Refresh,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    Claude,
    Codex,
    Gemini,
    Cursor,
}

impl AgentKind {
    pub fn id(&self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Gemini => "gemini",
            AgentKind::Cursor => "cursor",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude",
            AgentKind::Codex => "Codex",
            AgentKind::Gemini => "Gemini",
            AgentKind::Cursor => "Cursor",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            AgentKind::Claude => Color::rgb(192, 132, 252),
            AgentKind::Codex => Color::rgb(52, 211, 153),
            AgentKind::Gemini => Color::rgb(96, 165, 250),
            AgentKind::Cursor => Color::rgb(0, 212, 255),
        }
    }
}

#[derive(Clone)]
pub struct BtnArea {
    pub btn: ActionBtn,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub icon_x: i32,
    pub icon_y: i32,
}

#[derive(Clone)]
pub struct LogoArea {
    pub kind: AgentKind,
    pub name: &'static str,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub tooltip_anchor: (i32, i32),
}

#[derive(Clone)]
pub struct AgentSlot {
    pub kind: AgentKind,
    pub logo_pos: (i32, i32),
    pub gauge_pos: (i32, i32),
    pub hover_area: LogoArea,
}

pub struct LayoutMetrics {
    pub win_w: usize,
    pub win_h: usize,
    pub grip_pos: (i32, i32),
    pub agent_slots: Vec<AgentSlot>,
    pub dividers: Vec<i32>,
    pub btn_divider: Option<i32>,
    pub buttons: Vec<BtnArea>,
}

pub fn get_active_kinds(cfg: &AppConfig) -> Vec<AgentKind> {
    let all = [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::Gemini,
        AgentKind::Cursor,
    ];
    let mut res = Vec::new();
    for &k in &all {
        if cfg.enabled_agents.contains(&k.id().to_string()) {
            res.push(k);
        }
    }
    if res.is_empty() {
        all.to_vec()
    } else {
        res
    }
}

pub fn compute_layout(cfg: &AppConfig, is_hovered: bool) -> LayoutMetrics {
    let active_kinds = get_active_kinds(cfg);
    let k_count = active_kinds.len();

    match cfg.orientation {
        Orientation::Horizontal => match cfg.scale {
            ScaleSize::Small => {
                let win_h = 30;
                let item_w = 47;
                let start_base_x = 12;

                let mut slots = Vec::new();
                let mut dividers = Vec::new();

                for (i, &kind) in active_kinds.iter().enumerate() {
                    let sx = start_base_x + (i as i32) * item_w;
                    slots.push(AgentSlot {
                        kind,
                        logo_pos: (sx + 4, 10),
                        gauge_pos: (sx + 28, 15),
                        hover_area: LogoArea {
                            kind,
                            name: kind.name(),
                            x: sx,
                            y: 2,
                            w: item_w - 1,
                            h: 26,
                            tooltip_anchor: (sx + 9, 6),
                        },
                    });
                    if i < k_count - 1 {
                        dividers.push(sx + item_w);
                    }
                }

                let compact_w = (start_base_x + (k_count as i32) * item_w + 4) as usize;
                let (win_w, btn_div, buttons) = if is_hovered {
                    let mut btns = Vec::new();
                    let btn_actions = [
                        ActionBtn::Orientation,
                        ActionBtn::Resize,
                        ActionBtn::Settings,
                        ActionBtn::Refresh,
                        ActionBtn::Close,
                    ];
                    let bx_start = (compact_w as i32) + 3;
                    for (b, &action) in btn_actions.iter().enumerate() {
                        let bx = bx_start + (b as i32) * 11;
                        btns.push(BtnArea {
                            btn: action,
                            x: bx,
                            y: 2,
                            w: 11,
                            h: 26,
                            icon_x: bx + 2,
                            icon_y: 11,
                        });
                    }
                    (
                        (bx_start + (btn_actions.len() as i32) * 11 + 6) as usize,
                        Some((compact_w as i32) - 2),
                        btns,
                    )
                } else {
                    (compact_w, None, Vec::new())
                };

                LayoutMetrics {
                    win_w,
                    win_h,
                    grip_pos: (6, 10),
                    agent_slots: slots,
                    dividers,
                    btn_divider: btn_div,
                    buttons,
                }
            }
            ScaleSize::Normal => {
                let win_h = 40;
                let item_w = 64;
                let start_base_x = 16;

                let mut slots = Vec::new();
                let mut dividers = Vec::new();

                for (i, &kind) in active_kinds.iter().enumerate() {
                    let sx = start_base_x + (i as i32) * item_w;
                    slots.push(AgentSlot {
                        kind,
                        logo_pos: (sx + 4, 13),
                        gauge_pos: (sx + 37, 20),
                        hover_area: LogoArea {
                            kind,
                            name: kind.name(),
                            x: sx,
                            y: 2,
                            w: item_w - 1,
                            h: 36,
                            tooltip_anchor: (sx + 11, 8),
                        },
                    });
                    if i < k_count - 1 {
                        dividers.push(sx + item_w);
                    }
                }

                let compact_w = (start_base_x + (k_count as i32) * item_w + 6) as usize;
                let (win_w, btn_div, buttons) = if is_hovered {
                    let mut btns = Vec::new();
                    let btn_actions = [
                        ActionBtn::Orientation,
                        ActionBtn::Resize,
                        ActionBtn::Settings,
                        ActionBtn::Refresh,
                        ActionBtn::Close,
                    ];
                    let bx_start = (compact_w as i32) + 4;
                    for (b, &action) in btn_actions.iter().enumerate() {
                        let bx = bx_start + (b as i32) * 14;
                        btns.push(BtnArea {
                            btn: action,
                            x: bx,
                            y: 2,
                            w: 14,
                            h: 36,
                            icon_x: bx + 2,
                            icon_y: 15,
                        });
                    }
                    (
                        (bx_start + (btn_actions.len() as i32) * 14 + 7) as usize,
                        Some((compact_w as i32) - 2),
                        btns,
                    )
                } else {
                    (compact_w, None, Vec::new())
                };

                LayoutMetrics {
                    win_w,
                    win_h,
                    grip_pos: (7, 15),
                    agent_slots: slots,
                    dividers,
                    btn_divider: btn_div,
                    buttons,
                }
            }
            ScaleSize::Large => {
                let win_h = 58;
                let item_w = 90;
                let start_base_x = 24;

                let mut slots = Vec::new();
                let mut dividers = Vec::new();

                for (i, &kind) in active_kinds.iter().enumerate() {
                    let sx = start_base_x + (i as i32) * item_w;
                    slots.push(AgentSlot {
                        kind,
                        logo_pos: (sx + 4, 19),
                        gauge_pos: (sx + 52, 29),
                        hover_area: LogoArea {
                            kind,
                            name: kind.name(),
                            x: sx,
                            y: 4,
                            w: item_w - 1,
                            h: 50,
                            tooltip_anchor: (sx + 14, 12),
                        },
                    });
                    if i < k_count - 1 {
                        dividers.push(sx + item_w);
                    }
                }

                let compact_w = (start_base_x + (k_count as i32) * item_w + 8) as usize;
                let (win_w, btn_div, buttons) = if is_hovered {
                    let mut btns = Vec::new();
                    let btn_actions = [
                        ActionBtn::Orientation,
                        ActionBtn::Resize,
                        ActionBtn::Settings,
                        ActionBtn::Refresh,
                        ActionBtn::Close,
                    ];
                    let bx_start = (compact_w as i32) + 5;
                    for (b, &action) in btn_actions.iter().enumerate() {
                        let bx = bx_start + (b as i32) * 20;
                        btns.push(BtnArea {
                            btn: action,
                            x: bx,
                            y: 2,
                            w: 20,
                            h: 54,
                            icon_x: bx + 3,
                            icon_y: 22,
                        });
                    }
                    (
                        (bx_start + (btn_actions.len() as i32) * 20 + 9) as usize,
                        Some((compact_w as i32) - 2),
                        btns,
                    )
                } else {
                    (compact_w, None, Vec::new())
                };

                LayoutMetrics {
                    win_w,
                    win_h,
                    grip_pos: (9, 19),
                    agent_slots: slots,
                    dividers,
                    btn_divider: btn_div,
                    buttons,
                }
            }
        },
        Orientation::Vertical => match cfg.scale {
            ScaleSize::Small => {
                let win_w = 42;
                let item_h = 40;
                let start_base_y = 10;

                let mut slots = Vec::new();
                let mut dividers = Vec::new();

                for (i, &kind) in active_kinds.iter().enumerate() {
                    let sy = start_base_y + (i as i32) * item_h;
                    slots.push(AgentSlot {
                        kind,
                        logo_pos: (16, sy + 4),
                        gauge_pos: (21, sy + 25),
                        hover_area: LogoArea {
                            kind,
                            name: kind.name(),
                            x: 2,
                            y: sy,
                            w: 38,
                            h: item_h - 1,
                            tooltip_anchor: (21, sy),
                        },
                    });
                    if i < k_count - 1 {
                        dividers.push(sy + item_h);
                    }
                }

                let compact_h = (start_base_y + (k_count as i32) * item_h + 4) as usize;
                let (win_h, btn_div, buttons) = if is_hovered {
                    let mut btns = Vec::new();
                    let btn_actions = [
                        ActionBtn::Orientation,
                        ActionBtn::Resize,
                        ActionBtn::Settings,
                        ActionBtn::Refresh,
                        ActionBtn::Close,
                    ];
                    let by_start = (compact_h as i32) + 3;
                    for (b, &action) in btn_actions.iter().enumerate() {
                        let by = by_start + (b as i32) * 11;
                        btns.push(BtnArea {
                            btn: action,
                            x: 2,
                            y: by,
                            w: 38,
                            h: 11,
                            icon_x: 17,
                            icon_y: by + 2,
                        });
                    }
                    (
                        (by_start + (btn_actions.len() as i32) * 11 + 6) as usize,
                        Some((compact_h as i32) - 2),
                        btns,
                    )
                } else {
                    (compact_h, None, Vec::new())
                };

                LayoutMetrics {
                    win_w,
                    win_h,
                    grip_pos: (16, 6),
                    agent_slots: slots,
                    dividers,
                    btn_divider: btn_div,
                    buttons,
                }
            }
            ScaleSize::Normal => {
                let win_w = 48;
                let item_h = 52;
                let start_base_y = 12;

                let mut slots = Vec::new();
                let mut dividers = Vec::new();

                for (i, &kind) in active_kinds.iter().enumerate() {
                    let sy = start_base_y + (i as i32) * item_h;
                    slots.push(AgentSlot {
                        kind,
                        logo_pos: (17, sy + 6),
                        gauge_pos: (24, sy + 34),
                        hover_area: LogoArea {
                            kind,
                            name: kind.name(),
                            x: 2,
                            y: sy,
                            w: 44,
                            h: item_h - 1,
                            tooltip_anchor: (24, sy + 2),
                        },
                    });
                    if i < k_count - 1 {
                        dividers.push(sy + item_h);
                    }
                }

                let compact_h = (start_base_y + (k_count as i32) * item_h + 6) as usize;
                let (win_h, btn_div, buttons) = if is_hovered {
                    let mut btns = Vec::new();
                    let btn_actions = [
                        ActionBtn::Orientation,
                        ActionBtn::Resize,
                        ActionBtn::Settings,
                        ActionBtn::Refresh,
                        ActionBtn::Close,
                    ];
                    let by_start = (compact_h as i32) + 4;
                    for (b, &action) in btn_actions.iter().enumerate() {
                        let by = by_start + (b as i32) * 14;
                        btns.push(BtnArea {
                            btn: action,
                            x: 2,
                            y: by,
                            w: 44,
                            h: 14,
                            icon_x: 19,
                            icon_y: by + 2,
                        });
                    }
                    (
                        (by_start + (btn_actions.len() as i32) * 14 + 7) as usize,
                        Some((compact_h as i32) - 2),
                        btns,
                    )
                } else {
                    (compact_h, None, Vec::new())
                };

                LayoutMetrics {
                    win_w,
                    win_h,
                    grip_pos: (19, 7),
                    agent_slots: slots,
                    dividers,
                    btn_divider: btn_div,
                    buttons,
                }
            }
            ScaleSize::Large => {
                let win_w = 66;
                let item_h = 75;
                let start_base_y = 18;

                let mut slots = Vec::new();
                let mut dividers = Vec::new();

                for (i, &kind) in active_kinds.iter().enumerate() {
                    let sy = start_base_y + (i as i32) * item_h;
                    slots.push(AgentSlot {
                        kind,
                        logo_pos: (23, sy + 7),
                        gauge_pos: (33, sy + 48),
                        hover_area: LogoArea {
                            kind,
                            name: kind.name(),
                            x: 2,
                            y: sy,
                            w: 62,
                            h: item_h - 1,
                            tooltip_anchor: (33, sy + 2),
                        },
                    });
                    if i < k_count - 1 {
                        dividers.push(sy + item_h);
                    }
                }

                let compact_h = (start_base_y + (k_count as i32) * item_h + 8) as usize;
                let (win_h, btn_div, buttons) = if is_hovered {
                    let mut btns = Vec::new();
                    let btn_actions = [
                        ActionBtn::Orientation,
                        ActionBtn::Resize,
                        ActionBtn::Settings,
                        ActionBtn::Refresh,
                        ActionBtn::Close,
                    ];
                    let by_start = (compact_h as i32) + 5;
                    for (b, &action) in btn_actions.iter().enumerate() {
                        let by = by_start + (b as i32) * 20;
                        btns.push(BtnArea {
                            btn: action,
                            x: 2,
                            y: by,
                            w: 62,
                            h: 20,
                            icon_x: 26,
                            icon_y: by + 3,
                        });
                    }
                    (
                        (by_start + (btn_actions.len() as i32) * 20 + 9) as usize,
                        Some((compact_h as i32) - 2),
                        btns,
                    )
                } else {
                    (compact_h, None, Vec::new())
                };

                LayoutMetrics {
                    win_w,
                    win_h,
                    grip_pos: (23, 9),
                    agent_slots: slots,
                    dividers,
                    btn_divider: btn_div,
                    buttons,
                }
            }
        },
    }
}

fn draw_widget(
    fb: &mut Framebuffer,
    data: &QuotaSummary,
    cfg: &AppConfig,
    is_hovered: bool,
    hovered_btn: Option<ActionBtn>,
    hovered_logo: Option<AgentKind>,
) {
    let layout = compute_layout(cfg, is_hovered);
    fb.resize(layout.win_w, layout.win_h);
    fb.clear(Color::rgb(12, 16, 23));

    let is_vert = cfg.orientation == Orientation::Vertical;

    // 1. Drag grip
    fb.draw_drag_grip(layout.grip_pos.0, layout.grip_pos.1, is_vert, cfg.scale);

    // 2. Active Agent Slots
    for slot in &layout.agent_slots {
        let pct = match slot.kind {
            AgentKind::Claude => data.claude_5h,
            AgentKind::Codex => data.codex_5h,
            AgentKind::Gemini => data.gemini,
            AgentKind::Cursor => data.cursor,
        };

        match slot.kind {
            AgentKind::Claude => fb.draw_claude_logo(slot.logo_pos.0, slot.logo_pos.1, cfg.scale),
            AgentKind::Codex => fb.draw_codex_logo(slot.logo_pos.0, slot.logo_pos.1, cfg.scale),
            AgentKind::Gemini => fb.draw_gemini_logo(slot.logo_pos.0, slot.logo_pos.1, cfg.scale),
            AgentKind::Cursor => fb.draw_cursor_logo(slot.logo_pos.0, slot.logo_pos.1, cfg.scale),
        }

        fb.draw_ring_gauge(
            slot.gauge_pos.0,
            slot.gauge_pos.1,
            pct,
            slot.kind.color(),
            cfg.scale,
        );
    }

    // 3. Dividers between agents
    for &pos in &layout.dividers {
        if is_vert {
            fb.draw_divider_h(pos);
        } else {
            fb.draw_divider_v(pos);
        }
    }

    // 4. Action buttons (when hovered)
    if is_hovered {
        if let Some(pos) = layout.btn_divider {
            if is_vert {
                fb.draw_divider_h(pos);
            } else {
                fb.draw_divider_v(pos);
            }
        }

        for area in &layout.buttons {
            let is_target = hovered_btn == Some(area.btn);
            if is_target {
                fb.fill_rect(
                    area.x + 1,
                    area.y + 1,
                    (area.w - 2).max(1),
                    (area.h - 2).max(1),
                    Color::rgb(28, 38, 56),
                );
            }

            match area.btn {
                ActionBtn::Orientation => {
                    fb.draw_icon_orientation(area.icon_x, area.icon_y, is_target, cfg.scale);
                }
                ActionBtn::Resize => {
                    fb.draw_icon_resize(area.icon_x, area.icon_y, is_target, cfg.scale);
                }
                ActionBtn::Settings => {
                    fb.draw_icon_settings(area.icon_x, area.icon_y, is_target, cfg.scale);
                }
                ActionBtn::Refresh => {
                    fb.draw_icon_refresh(area.icon_x, area.icon_y, is_target, cfg.scale);
                }
                ActionBtn::Close => {
                    fb.draw_icon_close(area.icon_x, area.icon_y, is_target, cfg.scale);
                }
            }
        }
    }

    // 5. Logo Tooltip (when logo hovered)
    if let Some(target) = hovered_logo {
        if let Some(slot) = layout.agent_slots.iter().find(|s| s.kind == target) {
            fb.draw_tooltip(
                slot.hover_area.tooltip_anchor.0,
                slot.hover_area.tooltip_anchor.1,
                slot.kind.name(),
                cfg.scale,
            );
        }
    }

    // Outer capsule border
    fb.draw_rounded_capsule_outline(Color::rgb(38, 48, 66));
}

fn draw_settings_window(
    fb: &mut Framebuffer,
    cfg: &AppConfig,
    hovered_row: Option<usize>,
    hovered_close: bool,
) {
    fb.resize(180, 180);
    fb.clear(Color::rgb(14, 18, 27));

    // Title
    fb.draw_text_scaled(12, 10, "ACTIVE AGENTS", Color::rgb(226, 232, 240), 1);

    // Close button (X)
    let close_color = if hovered_close {
        Color::rgb(244, 63, 94)
    } else {
        Color::rgb(148, 163, 184)
    };
    for i in 0..7 {
        fb.set_pixel(160 + i, 10 + i, close_color);
        fb.set_pixel(160 + 6 - i, 10 + i, close_color);
    }

    // Divider below title
    for x in 8..172 {
        fb.set_pixel(x, 24, Color::rgb(35, 45, 62));
    }

    // 4 Checkbox rows
    let all_kinds = [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::Gemini,
        AgentKind::Cursor,
    ];

    for (i, &kind) in all_kinds.iter().enumerate() {
        let row_y = 30 + (i as i32) * 28;
        let is_hover = hovered_row == Some(i);
        let checked = cfg.enabled_agents.contains(&kind.id().to_string());
        fb.draw_checkbox_row(
            4,
            row_y,
            172,
            26,
            checked,
            is_hover,
            kind.name(),
            kind.color(),
        );
    }

    // Divider above footer
    for x in 8..172 {
        fb.set_pixel(x, 148, Color::rgb(35, 45, 62));
    }

    // Footer note
    fb.draw_text_3x5(12, 156, "CLICK TO TOGGLE | SAVED", Color::rgb(100, 116, 139));

    // Card border
    fb.draw_rounded_capsule_outline(Color::rgb(51, 65, 85));
}

fn upload_frame(
    conn: &RustConnection,
    win: Window,
    gc: Gcontext,
    fb: &Framebuffer,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut data = Vec::with_capacity(fb.pixels.len() * 4);
    for &px in &fb.pixels {
        data.extend_from_slice(&px.to_ne_bytes());
    }

    conn.put_image(
        ImageFormat::Z_PIXMAP,
        win,
        gc,
        fb.width as u16,
        fb.height as u16,
        0,
        0,
        0,
        24,
        &data,
    )?;
    conn.flush()?;
    Ok(())
}

fn trigger_background_fetch(tx: Sender<QuotaSummary>) {
    thread::spawn(move || {
        let summary = fetch_all();
        let _ = tx.send(summary);
    });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("[quota-ui] Starting ultra-compact AI Quota Ring Meter...");

    let (conn, screen_num) = RustConnection::connect(None)?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;

    let mut cfg = load_config();
    let mut is_hovered = false;
    let mut hovered_btn: Option<ActionBtn> = None;
    let mut hovered_logo: Option<AgentKind> = None;

    // Hover debounce / hysteresis: prevents flickering on edge crossings
    let mut leave_deadline: Option<Instant> = None;
    let hover_grace_period = Duration::from_millis(250);

    let initial_layout = compute_layout(&cfg, is_hovered);

    // 1. Main HUD Window
    let win = conn.generate_id()?;
    let win_aux = CreateWindowAux::new()
        .background_pixel(Color::rgb(12, 16, 23).to_x11_pixel())
        .override_redirect(1) // Borderless floating HUD window
        .event_mask(
            EventMask::EXPOSURE
                | EventMask::BUTTON_PRESS
                | EventMask::BUTTON_RELEASE
                | EventMask::POINTER_MOTION
                | EventMask::ENTER_WINDOW
                | EventMask::LEAVE_WINDOW
                | EventMask::KEY_PRESS,
        );

    conn.create_window(
        24,
        win,
        root,
        cfg.x as i16,
        cfg.y as i16,
        initial_layout.win_w as u16,
        initial_layout.win_h as u16,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &win_aux,
    )?;

    // Set Window Title
    let title = "AI Quota HUD";
    conn.change_property8(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        title.as_bytes(),
    )?;

    // Set _NET_WM_STATE to ABOVE (Always on top)
    let net_wm_state = conn.intern_atom(false, b"_NET_WM_STATE")?.reply()?.atom;
    let net_wm_state_above = conn
        .intern_atom(false, b"_NET_WM_STATE_ABOVE")?
        .reply()?
        .atom;
    conn.change_property32(
        PropMode::REPLACE,
        win,
        net_wm_state,
        AtomEnum::ATOM,
        &[net_wm_state_above],
    )?;

    let net_wm_window_type = conn
        .intern_atom(false, b"_NET_WM_WINDOW_TYPE")?
        .reply()?
        .atom;
    let net_wm_type_utility = conn
        .intern_atom(false, b"_NET_WM_WINDOW_TYPE_UTILITY")?
        .reply()?
        .atom;
    conn.change_property32(
        PropMode::REPLACE,
        win,
        net_wm_window_type,
        AtomEnum::ATOM,
        &[net_wm_type_utility],
    )?;

    let gc = conn.generate_id()?;
    conn.create_gc(gc, win, &CreateGCAux::new())?;

    conn.map_window(win)?;

    // 2. Settings Modal Window
    let settings_win = conn.generate_id()?;
    let settings_aux = CreateWindowAux::new()
        .background_pixel(Color::rgb(14, 18, 27).to_x11_pixel())
        .override_redirect(1)
        .event_mask(
            EventMask::EXPOSURE
                | EventMask::BUTTON_PRESS
                | EventMask::POINTER_MOTION
                | EventMask::LEAVE_WINDOW,
        );

    conn.create_window(
        24,
        settings_win,
        root,
        (cfg.x + 20) as i16,
        (cfg.y + (initial_layout.win_h as i32) + 8) as i16,
        180,
        180,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &settings_aux,
    )?;

    let settings_title = "Quota Settings";
    conn.change_property8(
        PropMode::REPLACE,
        settings_win,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        settings_title.as_bytes(),
    )?;
    conn.change_property32(
        PropMode::REPLACE,
        settings_win,
        net_wm_state,
        AtomEnum::ATOM,
        &[net_wm_state_above],
    )?;

    let settings_gc = conn.generate_id()?;
    conn.create_gc(settings_gc, settings_win, &CreateGCAux::new())?;

    let mut settings_open = false;
    let mut settings_hovered_row: Option<usize> = None;
    let mut settings_hovered_close = false;
    let mut settings_fb = Framebuffer::new(180, 180);

    conn.flush()?;

    let mut fb = Framebuffer::new(initial_layout.win_w, initial_layout.win_h);
    let current_data = Arc::new(Mutex::new(QuotaSummary::default()));

    // Channel for background quota updates
    let (tx, rx): (Sender<QuotaSummary>, Receiver<QuotaSummary>) = channel();
    trigger_background_fetch(tx.clone());

    // State
    let mut is_dragging = false;
    let mut drag_start_cursor_x = 0;
    let mut drag_start_cursor_y = 0;
    let mut drag_start_win_x = cfg.x;
    let mut drag_start_win_y = cfg.y;

    let mut last_poll = Instant::now();
    let poll_interval = Duration::from_secs(300); // 5 minutes

    // Initial render
    {
        let data = current_data.lock().unwrap();
        draw_widget(
            &mut fb,
            &data,
            &cfg,
            is_hovered,
            hovered_btn,
            hovered_logo,
        );
    }
    upload_frame(&conn, win, gc, &fb)?;

    'main_loop: loop {
        // 1. Background fetch result
        if let Ok(fresh) = rx.try_recv() {
            {
                let mut data = current_data.lock().unwrap();
                *data = fresh;
                draw_widget(
                    &mut fb,
                    &data,
                    &cfg,
                    is_hovered,
                    hovered_btn,
                    hovered_logo,
                );
            }
            upload_frame(&conn, win, gc, &fb)?;
            last_poll = Instant::now();
        }

        // 2. Periodic poll
        if last_poll.elapsed() >= poll_interval {
            last_poll = Instant::now();
            trigger_background_fetch(tx.clone());
        }

        // 3. Hover Debounce check
        if let Some(deadline) = leave_deadline {
            if Instant::now() >= deadline {
                leave_deadline = None;
                if is_hovered && !is_dragging {
                    is_hovered = false;
                    hovered_btn = None;
                    hovered_logo = None;
                    let layout = compute_layout(&cfg, is_hovered);
                    conn.configure_window(
                        win,
                        &ConfigureWindowAux::new()
                            .width(layout.win_w as u32)
                            .height(layout.win_h as u32),
                    )?;
                    conn.flush()?;

                    let data = current_data.lock().unwrap();
                    draw_widget(
                        &mut fb,
                        &data,
                        &cfg,
                        is_hovered,
                        None,
                        None,
                    );
                    upload_frame(&conn, win, gc, &fb)?;
                }
            }
        }

        // 4. Process X11 Events
        while let Some(event) = conn.poll_for_event()? {
            match event {
                x11rb::protocol::Event::Expose(ev) => {
                    if ev.window == win {
                        let data = current_data.lock().unwrap();
                        draw_widget(
                            &mut fb,
                            &data,
                            &cfg,
                            is_hovered,
                            hovered_btn,
                            hovered_logo,
                        );
                        upload_frame(&conn, win, gc, &fb)?;
                    } else if ev.window == settings_win && settings_open {
                        draw_settings_window(
                            &mut settings_fb,
                            &cfg,
                            settings_hovered_row,
                            settings_hovered_close,
                        );
                        upload_frame(&conn, settings_win, settings_gc, &settings_fb)?;
                    }
                }
                x11rb::protocol::Event::EnterNotify(ev) => {
                                        if ev.event == win {
                        if ev.detail == NotifyDetail::INFERIOR {
                            continue;
                        }
                        leave_deadline = None;
                        if !is_hovered {
                            is_hovered = true;
                            let layout = compute_layout(&cfg, is_hovered);
                            conn.configure_window(
                                win,
                                &ConfigureWindowAux::new()
                                    .width(layout.win_w as u32)
                                    .height(layout.win_h as u32),
                            )?;
                            conn.flush()?;

                            let data = current_data.lock().unwrap();
                            draw_widget(
                                &mut fb,
                                &data,
                                &cfg,
                                is_hovered,
                                hovered_btn,
                                hovered_logo,
                            );
                            upload_frame(&conn, win, gc, &fb)?;
                        }
                    }
                }
                x11rb::protocol::Event::LeaveNotify(ev) => {
                    if ev.event == win {
                        if ev.detail == NotifyDetail::INFERIOR {
                            continue;
                        }
                        if !is_dragging && is_hovered {
                            hovered_btn = None;
                            hovered_logo = None;
                            leave_deadline = Some(Instant::now() + hover_grace_period);

                            let data = current_data.lock().unwrap();
                            draw_widget(
                                &mut fb,
                                &data,
                                &cfg,
                                is_hovered,
                                None,
                                None,
                            );
                            upload_frame(&conn, win, gc, &fb)?;
                        }
                    } else if ev.event == settings_win {
                        settings_hovered_row = None;
                        settings_hovered_close = false;
                        if settings_open {
                            draw_settings_window(
                                &mut settings_fb,
                                &cfg,
                                None,
                                false,
                            );
                            upload_frame(&conn, settings_win, settings_gc, &settings_fb)?;
                        }
                    }
                }
                x11rb::protocol::Event::MotionNotify(ev) => {
                    if ev.event == win {
                        if is_dragging {
                            let dx = ev.root_x as i32 - drag_start_cursor_x;
                            let dy = ev.root_y as i32 - drag_start_cursor_y;
                            cfg.x = drag_start_win_x + dx;
                            cfg.y = drag_start_win_y + dy;

                            conn.configure_window(
                                win,
                                &ConfigureWindowAux::new().x(cfg.x).y(cfg.y),
                            )?;
                            if settings_open {
                                let layout = compute_layout(&cfg, is_hovered);
                                let sx = (cfg.x + 20).max(10);
                                let sy = (cfg.y + (layout.win_h as i32) + 8).max(10);
                                conn.configure_window(
                                    settings_win,
                                    &ConfigureWindowAux::new().x(sx).y(sy),
                                )?;
                            }
                            conn.flush()?;
                        } else {
                            leave_deadline = None;
                            let need_expand = !is_hovered;
                            if need_expand {
                                is_hovered = true;
                                let new_layout = compute_layout(&cfg, is_hovered);
                                conn.configure_window(
                                    win,
                                    &ConfigureWindowAux::new()
                                        .width(new_layout.win_w as u32)
                                        .height(new_layout.win_h as u32),
                                )?;
                                conn.flush()?;
                            }

                            let layout = compute_layout(&cfg, is_hovered);
                            let mx = ev.event_x as i32;
                            let my = ev.event_y as i32;

                            let mut target_btn = None;
                            for a in &layout.buttons {
                                if mx >= a.x && mx < a.x + a.w && my >= a.y && my < a.y + a.h {
                                    target_btn = Some(a.btn);
                                    break;
                                }
                            }

                            let mut target_logo = None;
                            for s in &layout.agent_slots {
                                let a = &s.hover_area;
                                if mx >= a.x && mx < a.x + a.w && my >= a.y && my < a.y + a.h {
                                    target_logo = Some(s.kind);
                                    break;
                                }
                            }

                            if target_btn != hovered_btn
                                || target_logo != hovered_logo
                                || need_expand
                            {
                                hovered_btn = target_btn;
                                hovered_logo = target_logo;

                                let data = current_data.lock().unwrap();
                                draw_widget(
                                    &mut fb,
                                    &data,
                                    &cfg,
                                    is_hovered,
                                    hovered_btn,
                                    hovered_logo,
                                );
                                upload_frame(&conn, win, gc, &fb)?;
                            }
                        }
                    } else if ev.event == settings_win && settings_open {
                        let mx = ev.event_x as i32;
                        let my = ev.event_y as i32;

                        let is_close = mx >= 155 && mx <= 172 && my >= 6 && my <= 20;
                        let mut target_row = None;
                        for i in 0..4 {
                            let ry = 30 + (i as i32) * 28;
                            if mx >= 4 && mx <= 176 && my >= ry && my < ry + 26 {
                                target_row = Some(i);
                                break;
                            }
                        }

                        if target_row != settings_hovered_row || is_close != settings_hovered_close {
                            settings_hovered_row = target_row;
                            settings_hovered_close = is_close;
                            draw_settings_window(
                                &mut settings_fb,
                                &cfg,
                                settings_hovered_row,
                                settings_hovered_close,
                            );
                            upload_frame(&conn, settings_win, settings_gc, &settings_fb)?;
                        }
                    }
                }
                x11rb::protocol::Event::ButtonPress(ev) => {
                    if ev.event == win && ev.detail == 1 {
                        if let Some(btn) = hovered_btn {
                            match btn {
                                ActionBtn::Orientation => {
                                    cfg.orientation = match cfg.orientation {
                                        Orientation::Horizontal => Orientation::Vertical,
                                        Orientation::Vertical => Orientation::Horizontal,
                                    };
                                    let layout = compute_layout(&cfg, is_hovered);
                                    conn.configure_window(
                                        win,
                                        &ConfigureWindowAux::new()
                                            .width(layout.win_w as u32)
                                            .height(layout.win_h as u32),
                                    )?;
                                    conn.flush()?;
                                    save_config(&cfg);

                                    let data = current_data.lock().unwrap();
                                    draw_widget(
                                        &mut fb,
                                        &data,
                                        &cfg,
                                        is_hovered,
                                        hovered_btn,
                                        hovered_logo,
                                    );
                                    upload_frame(&conn, win, gc, &fb)?;
                                }
                                ActionBtn::Resize => {
                                    cfg.scale = cfg.scale.next();
                                    let layout = compute_layout(&cfg, is_hovered);
                                    conn.configure_window(
                                        win,
                                        &ConfigureWindowAux::new()
                                            .width(layout.win_w as u32)
                                            .height(layout.win_h as u32),
                                    )?;
                                    conn.flush()?;
                                    save_config(&cfg);

                                    let data = current_data.lock().unwrap();
                                    draw_widget(
                                        &mut fb,
                                        &data,
                                        &cfg,
                                        is_hovered,
                                        hovered_btn,
                                        hovered_logo,
                                    );
                                    upload_frame(&conn, win, gc, &fb)?;
                                }
                                ActionBtn::Settings => {
                                    settings_open = !settings_open;
                                    if settings_open {
                                        let layout = compute_layout(&cfg, is_hovered);
                                        let sx = (cfg.x + 10).max(10);
                                        let sy = (cfg.y + (layout.win_h as i32) + 8).max(10);
                                        conn.configure_window(
                                            settings_win,
                                            &ConfigureWindowAux::new().x(sx).y(sy),
                                        )?;
                                        conn.map_window(settings_win)?;
                                        draw_settings_window(
                                            &mut settings_fb,
                                            &cfg,
                                            settings_hovered_row,
                                            settings_hovered_close,
                                        );
                                        upload_frame(
                                            &conn,
                                            settings_win,
                                            settings_gc,
                                            &settings_fb,
                                        )?;
                                    } else {
                                        conn.unmap_window(settings_win)?;
                                    }
                                }
                                ActionBtn::Refresh => {
                                    println!("[quota-ui] Manual refresh clicked");
                                    trigger_background_fetch(tx.clone());
                                }
                                ActionBtn::Close => {
                                    println!("[quota-ui] Close button clicked");
                                    break 'main_loop;
                                }
                            }
                        } else {
                            is_dragging = true;
                            drag_start_cursor_x = ev.root_x as i32;
                            drag_start_cursor_y = ev.root_y as i32;
                            drag_start_win_x = cfg.x;
                            drag_start_win_y = cfg.y;
                        }
                    } else if ev.event == win && ev.detail == 3 {
                        println!("[quota-ui] Manual refresh triggered via right-click");
                        trigger_background_fetch(tx.clone());
                    } else if ev.event == settings_win && ev.detail == 1 {
                        let mx = ev.event_x as i32;
                        let my = ev.event_y as i32;

                        // Check Close button
                        if mx >= 155 && mx <= 172 && my >= 6 && my <= 20 {
                            settings_open = false;
                            conn.unmap_window(settings_win)?;
                        } else {
                            // Check row click
                            let all_kinds = [
                                AgentKind::Claude,
                                AgentKind::Codex,
                                AgentKind::Gemini,
                                AgentKind::Cursor,
                            ];
                            for (i, &kind) in all_kinds.iter().enumerate() {
                                let ry = 30 + (i as i32) * 28;
                                if mx >= 4 && mx <= 176 && my >= ry && my < ry + 26 {
                                    let id = kind.id().to_string();
                                    if cfg.enabled_agents.contains(&id) {
                                        // Keep at least 1 agent
                                        if cfg.enabled_agents.len() > 1 {
                                            cfg.enabled_agents.retain(|s| s != &id);
                                        }
                                    } else {
                                        cfg.enabled_agents.push(id);
                                    }

                                    save_config(&cfg);

                                    // Redraw settings
                                    draw_settings_window(
                                        &mut settings_fb,
                                        &cfg,
                                        settings_hovered_row,
                                        settings_hovered_close,
                                    );
                                    upload_frame(
                                        &conn,
                                        settings_win,
                                        settings_gc,
                                        &settings_fb,
                                    )?;

                                    // Resize and redraw main window
                                    let new_layout = compute_layout(&cfg, is_hovered);
                                    conn.configure_window(
                                        win,
                                        &ConfigureWindowAux::new()
                                            .width(new_layout.win_w as u32)
                                            .height(new_layout.win_h as u32),
                                    )?;
                                    conn.flush()?;

                                    let data = current_data.lock().unwrap();
                                    draw_widget(
                                        &mut fb,
                                        &data,
                                        &cfg,
                                        is_hovered,
                                        hovered_btn,
                                        hovered_logo,
                                    );
                                    upload_frame(&conn, win, gc, &fb)?;
                                    break;
                                }
                            }
                        }
                    }
                }
                x11rb::protocol::Event::ButtonRelease(ev) => {
                    if ev.event == win && ev.detail == 1 && is_dragging {
                        is_dragging = false;
                        save_config(&cfg);
                    }
                }
                x11rb::protocol::Event::KeyPress(ev) => {
                    if ev.detail == 9 {
                        break 'main_loop;
                    }
                    if ev.detail == 27 {
                        trigger_background_fetch(tx.clone());
                    }
                    if ev.detail == 32 {
                        cfg.orientation = match cfg.orientation {
                            Orientation::Horizontal => Orientation::Vertical,
                            Orientation::Vertical => Orientation::Horizontal,
                        };
                        let layout = compute_layout(&cfg, is_hovered);
                        conn.configure_window(
                            win,
                            &ConfigureWindowAux::new()
                                .width(layout.win_w as u32)
                                .height(layout.win_h as u32),
                        )?;
                        conn.flush()?;
                        save_config(&cfg);

                        let data = current_data.lock().unwrap();
                        draw_widget(
                            &mut fb,
                            &data,
                            &cfg,
                            is_hovered,
                            hovered_btn,
                            hovered_logo,
                        );
                        upload_frame(&conn, win, gc, &fb)?;
                    }
                }
                _ => {}
            }
        }

        thread::sleep(Duration::from_millis(16));
    }

    if settings_open {
        let _ = conn.unmap_window(settings_win);
    }
    save_config(&cfg);
    println!("[quota-ui] Exited cleanly.");
    Ok(())
}
