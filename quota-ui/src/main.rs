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
    Refresh,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoveredLogo {
    Claude,
    Codex,
    Gemini,
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
    pub logo: HoveredLogo,
    pub name: &'static str,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub tooltip_anchor: (i32, i32),
}

pub struct LayoutMetrics {
    pub win_w: usize,
    pub win_h: usize,
    pub grip_pos: (i32, i32),
    pub claude_logo: (i32, i32),
    pub claude_gauge: (i32, i32),
    pub codex_logo: (i32, i32),
    pub codex_gauge: (i32, i32),
    pub gemini_logo: (i32, i32),
    pub gemini_gauge: (i32, i32),
    pub dividers: Vec<i32>,
    pub btn_divider: Option<i32>,
    pub buttons: Vec<BtnArea>,
    pub logos: Vec<LogoArea>,
}

pub fn compute_layout(cfg: &AppConfig, is_hovered: bool) -> LayoutMetrics {
    match cfg.orientation {
        Orientation::Horizontal => match cfg.scale {
            ScaleSize::Small => {
                let win_h = 30;
                let (win_w, btn_div, buttons) = if is_hovered {
                    let btns = vec![
                        BtnArea { btn: ActionBtn::Orientation, x: 153, y: 2, w: 11, h: 26, icon_x: 155, icon_y: 11 },
                        BtnArea { btn: ActionBtn::Resize, x: 164, y: 2, w: 11, h: 26, icon_x: 166, icon_y: 11 },
                        BtnArea { btn: ActionBtn::Refresh, x: 175, y: 2, w: 11, h: 26, icon_x: 177, icon_y: 11 },
                        BtnArea { btn: ActionBtn::Close, x: 186, y: 2, w: 11, h: 26, icon_x: 188, icon_y: 11 },
                    ];
                    (202, Some(149), btns)
                } else {
                    (150, None, Vec::new())
                };
                LayoutMetrics {
                    win_w,
                    win_h,
                    grip_pos: (6, 10),
                    claude_logo: (16, 10),
                    claude_gauge: (40, 15),
                    codex_logo: (63, 10),
                    codex_gauge: (87, 15),
                    gemini_logo: (110, 10),
                    gemini_gauge: (134, 15),
                    dividers: vec![55, 102],
                    btn_divider: btn_div,
                    buttons,
                    logos: vec![
                        LogoArea { logo: HoveredLogo::Claude, name: "Claude", x: 12, y: 2, w: 42, h: 26, tooltip_anchor: (21, 6) },
                        LogoArea { logo: HoveredLogo::Codex, name: "Codex", x: 59, y: 2, w: 42, h: 26, tooltip_anchor: (68, 6) },
                        LogoArea { logo: HoveredLogo::Gemini, name: "Gemini", x: 106, y: 2, w: 42, h: 26, tooltip_anchor: (115, 6) },
                    ],
                }
            }
            ScaleSize::Normal => {
                let win_h = 40;
                let (win_w, btn_div, buttons) = if is_hovered {
                    let btns = vec![
                        BtnArea { btn: ActionBtn::Orientation, x: 206, y: 2, w: 14, h: 36, icon_x: 208, icon_y: 15 },
                        BtnArea { btn: ActionBtn::Resize, x: 220, y: 2, w: 14, h: 36, icon_x: 222, icon_y: 15 },
                        BtnArea { btn: ActionBtn::Refresh, x: 234, y: 2, w: 14, h: 36, icon_x: 236, icon_y: 15 },
                        BtnArea { btn: ActionBtn::Close, x: 248, y: 2, w: 14, h: 36, icon_x: 250, icon_y: 15 },
                    ];
                    (268, Some(201), btns)
                } else {
                    (202, None, Vec::new())
                };
                LayoutMetrics {
                    win_w,
                    win_h,
                    grip_pos: (7, 15),
                    claude_logo: (20, 13),
                    claude_gauge: (53, 20),
                    codex_logo: (84, 13),
                    codex_gauge: (117, 20),
                    gemini_logo: (148, 13),
                    gemini_gauge: (181, 20),
                    dividers: vec![75, 139],
                    btn_divider: btn_div,
                    buttons,
                    logos: vec![
                        LogoArea { logo: HoveredLogo::Claude, name: "Claude", x: 16, y: 2, w: 57, h: 36, tooltip_anchor: (27, 8) },
                        LogoArea { logo: HoveredLogo::Codex, name: "Codex", x: 80, y: 2, w: 57, h: 36, tooltip_anchor: (91, 8) },
                        LogoArea { logo: HoveredLogo::Gemini, name: "Gemini", x: 144, y: 2, w: 57, h: 36, tooltip_anchor: (155, 8) },
                    ],
                }
            }
            ScaleSize::Large => {
                let win_h = 58;
                let (win_w, btn_div, buttons) = if is_hovered {
                    let btns = vec![
                        BtnArea { btn: ActionBtn::Orientation, x: 293, y: 2, w: 20, h: 54, icon_x: 295, icon_y: 22 },
                        BtnArea { btn: ActionBtn::Resize, x: 313, y: 2, w: 20, h: 54, icon_x: 315, icon_y: 22 },
                        BtnArea { btn: ActionBtn::Refresh, x: 333, y: 2, w: 20, h: 54, icon_x: 335, icon_y: 22 },
                        BtnArea { btn: ActionBtn::Close, x: 353, y: 2, w: 20, h: 54, icon_x: 355, icon_y: 22 },
                    ];
                    (380, Some(287), btns)
                } else {
                    (288, None, Vec::new())
                };
                LayoutMetrics {
                    win_w,
                    win_h,
                    grip_pos: (9, 19),
                    claude_logo: (28, 19),
                    claude_gauge: (76, 29),
                    codex_logo: (119, 19),
                    codex_gauge: (167, 29),
                    gemini_logo: (210, 19),
                    gemini_gauge: (258, 29),
                    dividers: vec![107, 198],
                    btn_divider: btn_div,
                    buttons,
                    logos: vec![
                        LogoArea { logo: HoveredLogo::Claude, name: "Claude", x: 24, y: 4, w: 80, h: 50, tooltip_anchor: (38, 12) },
                        LogoArea { logo: HoveredLogo::Codex, name: "Codex", x: 115, y: 4, w: 80, h: 50, tooltip_anchor: (129, 12) },
                        LogoArea { logo: HoveredLogo::Gemini, name: "Gemini", x: 206, y: 4, w: 80, h: 50, tooltip_anchor: (220, 12) },
                    ],
                }
            }
        },
        Orientation::Vertical => match cfg.scale {
            ScaleSize::Small => {
                let win_w = 42; // Generous 42px width for comfort
                let (win_h, btn_div, buttons) = if is_hovered {
                    let btns = vec![
                        BtnArea { btn: ActionBtn::Orientation, x: 2, y: 133, w: 38, h: 11, icon_x: 17, icon_y: 135 },
                        BtnArea { btn: ActionBtn::Resize, x: 2, y: 144, w: 38, h: 11, icon_x: 17, icon_y: 146 },
                        BtnArea { btn: ActionBtn::Refresh, x: 2, y: 155, w: 38, h: 11, icon_x: 17, icon_y: 157 },
                        BtnArea { btn: ActionBtn::Close, x: 2, y: 166, w: 38, h: 11, icon_x: 17, icon_y: 168 },
                    ];
                    (182, Some(129), btns)
                } else {
                    (130, None, Vec::new())
                };
                LayoutMetrics {
                    win_w,
                    win_h,
                    grip_pos: (16, 6),
                    claude_logo: (16, 14),
                    claude_gauge: (21, 35),
                    codex_logo: (16, 54),
                    codex_gauge: (21, 75),
                    gemini_logo: (16, 94),
                    gemini_gauge: (21, 115),
                    dividers: vec![49, 89],
                    btn_divider: btn_div,
                    buttons,
                    logos: vec![
                        LogoArea { logo: HoveredLogo::Claude, name: "Claude", x: 2, y: 10, w: 38, h: 38, tooltip_anchor: (21, 10) },
                        LogoArea { logo: HoveredLogo::Codex, name: "Codex", x: 2, y: 50, w: 38, h: 38, tooltip_anchor: (21, 50) },
                        LogoArea { logo: HoveredLogo::Gemini, name: "Gemini", x: 2, y: 90, w: 38, h: 38, tooltip_anchor: (21, 90) },
                    ],
                }
            }
            ScaleSize::Normal => {
                let win_w = 48; // Generous 48px width
                let (win_h, btn_div, buttons) = if is_hovered {
                    let btns = vec![
                        BtnArea { btn: ActionBtn::Orientation, x: 2, y: 174, w: 44, h: 14, icon_x: 19, icon_y: 176 },
                        BtnArea { btn: ActionBtn::Resize, x: 2, y: 188, w: 44, h: 14, icon_x: 19, icon_y: 190 },
                        BtnArea { btn: ActionBtn::Refresh, x: 2, y: 202, w: 44, h: 14, icon_x: 19, icon_y: 204 },
                        BtnArea { btn: ActionBtn::Close, x: 2, y: 216, w: 44, h: 14, icon_x: 19, icon_y: 218 },
                    ];
                    (236, Some(169), btns)
                } else {
                    (170, None, Vec::new())
                };
                LayoutMetrics {
                    win_w,
                    win_h,
                    grip_pos: (19, 7),
                    claude_logo: (17, 18),
                    claude_gauge: (24, 46),
                    codex_logo: (17, 70),
                    codex_gauge: (24, 98),
                    gemini_logo: (17, 122),
                    gemini_gauge: (24, 150),
                    dividers: vec![64, 116],
                    btn_divider: btn_div,
                    buttons,
                    logos: vec![
                        LogoArea { logo: HoveredLogo::Claude, name: "Claude", x: 2, y: 12, w: 44, h: 50, tooltip_anchor: (24, 14) },
                        LogoArea { logo: HoveredLogo::Codex, name: "Codex", x: 2, y: 66, w: 44, h: 48, tooltip_anchor: (24, 66) },
                        LogoArea { logo: HoveredLogo::Gemini, name: "Gemini", x: 2, y: 118, w: 44, h: 48, tooltip_anchor: (24, 118) },
                    ],
                }
            }
            ScaleSize::Large => {
                let win_w = 66; // Generous 66px width
                let (win_h, btn_div, buttons) = if is_hovered {
                    let btns = vec![
                        BtnArea { btn: ActionBtn::Orientation, x: 2, y: 249, w: 62, h: 20, icon_x: 26, icon_y: 252 },
                        BtnArea { btn: ActionBtn::Resize, x: 2, y: 269, w: 62, h: 20, icon_x: 26, icon_y: 272 },
                        BtnArea { btn: ActionBtn::Refresh, x: 2, y: 289, w: 62, h: 20, icon_x: 26, icon_y: 292 },
                        BtnArea { btn: ActionBtn::Close, x: 2, y: 309, w: 62, h: 20, icon_x: 26, icon_y: 312 },
                    ];
                    (336, Some(243), btns)
                } else {
                    (244, None, Vec::new())
                };
                LayoutMetrics {
                    win_w,
                    win_h,
                    grip_pos: (23, 9),
                    claude_logo: (23, 25),
                    claude_gauge: (33, 66),
                    codex_logo: (23, 100),
                    codex_gauge: (33, 141),
                    gemini_logo: (23, 175),
                    gemini_gauge: (33, 216),
                    dividers: vec![92, 167],
                    btn_divider: btn_div,
                    buttons,
                    logos: vec![
                        LogoArea { logo: HoveredLogo::Claude, name: "Claude", x: 2, y: 18, w: 62, h: 72, tooltip_anchor: (33, 20) },
                        LogoArea { logo: HoveredLogo::Codex, name: "Codex", x: 2, y: 94, w: 62, h: 71, tooltip_anchor: (33, 95) },
                        LogoArea { logo: HoveredLogo::Gemini, name: "Gemini", x: 2, y: 169, w: 62, h: 71, tooltip_anchor: (33, 170) },
                    ],
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
    hovered_logo: Option<HoveredLogo>,
) {
    let layout = compute_layout(cfg, is_hovered);
    fb.resize(layout.win_w, layout.win_h);
    fb.clear(Color::rgb(12, 16, 23));

    let is_vert = cfg.orientation == Orientation::Vertical;

    // 1. Drag grip
    fb.draw_drag_grip(layout.grip_pos.0, layout.grip_pos.1, is_vert, cfg.scale);

    // 2. Claude
    fb.draw_claude_logo(layout.claude_logo.0, layout.claude_logo.1, cfg.scale);
    fb.draw_ring_gauge(
        layout.claude_gauge.0,
        layout.claude_gauge.1,
        data.claude_5h,
        Color::rgb(192, 132, 252),
        cfg.scale,
    );

    // 3. Codex
    fb.draw_codex_logo(layout.codex_logo.0, layout.codex_logo.1, cfg.scale);
    fb.draw_ring_gauge(
        layout.codex_gauge.0,
        layout.codex_gauge.1,
        data.codex_5h,
        Color::rgb(52, 211, 153),
        cfg.scale,
    );

    // 4. Gemini
    fb.draw_gemini_logo(layout.gemini_logo.0, layout.gemini_logo.1, cfg.scale);
    fb.draw_ring_gauge(
        layout.gemini_gauge.0,
        layout.gemini_gauge.1,
        data.gemini,
        Color::rgb(96, 165, 250),
        cfg.scale,
    );

    // 5. Dividers between agents
    for &pos in &layout.dividers {
        if is_vert {
            fb.draw_divider_h(pos);
        } else {
            fb.draw_divider_v(pos);
        }
    }

    // 6. Action buttons (when hovered)
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
                // Soft rounded highlight pill behind hovered button
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
                ActionBtn::Refresh => {
                    fb.draw_icon_refresh(area.icon_x, area.icon_y, is_target, cfg.scale);
                }
                ActionBtn::Close => {
                    fb.draw_icon_close(area.icon_x, area.icon_y, is_target, cfg.scale);
                }
            }
        }
    }

    // 7. Logo Tooltip (when logo hovered)
    if let Some(target) = hovered_logo {
        if let Some(area) = layout.logos.iter().find(|a| a.logo == target) {
            fb.draw_tooltip(area.tooltip_anchor.0, area.tooltip_anchor.1, area.name, cfg.scale);
        }
    }

    // Outer capsule border
    fb.draw_rounded_capsule_outline(Color::rgb(38, 48, 66));
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
    let mut hovered_logo: Option<HoveredLogo> = None;

    // Hover debounce / hysteresis: prevents flickering on edge crossings
    let mut leave_deadline: Option<Instant> = None;
    let hover_grace_period = Duration::from_millis(250);

    let initial_layout = compute_layout(&cfg, is_hovered);

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

    // Set _NET_WM_WINDOW_TYPE to UTILITY
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
        // 1. Check if background fetcher returned fresh quota
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

        // 2. Periodic re-fetch every 5 minutes
        if last_poll.elapsed() >= poll_interval {
            last_poll = Instant::now();
            trigger_background_fetch(tx.clone());
        }

        // 3. Check Hover Grace Period Debounce
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
                x11rb::protocol::Event::Expose(_) => {
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
                x11rb::protocol::Event::EnterNotify(ev) => {
                    if ev.detail == NotifyDetail::INFERIOR {
                        continue;
                    }
                    leave_deadline = None; // Cancel any scheduled leave collapse
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
                x11rb::protocol::Event::LeaveNotify(ev) => {
                    if ev.detail == NotifyDetail::INFERIOR {
                        continue;
                    }
                    if !is_dragging && is_hovered {
                        // Immediately clear button/logo hover highlights
                        hovered_btn = None;
                        hovered_logo = None;

                        // Start grace period before collapsing the window
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
                }
                x11rb::protocol::Event::MotionNotify(ev) => {
                    if is_dragging {
                        let dx = ev.root_x as i32 - drag_start_cursor_x;
                        let dy = ev.root_y as i32 - drag_start_cursor_y;
                        cfg.x = drag_start_win_x + dx;
                        cfg.y = drag_start_win_y + dy;

                        conn.configure_window(
                            win,
                            &ConfigureWindowAux::new().x(cfg.x).y(cfg.y),
                        )?;
                        conn.flush()?;
                    } else {
                        // Cursor is moving inside window: cancel any pending leave timer
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
                        for a in &layout.logos {
                            if mx >= a.x && mx < a.x + a.w && my >= a.y && my < a.y + a.h {
                                target_logo = Some(a.logo);
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
                }
                x11rb::protocol::Event::ButtonPress(ev) => {
                    if ev.detail == 1 {
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
                    } else if ev.detail == 3 {
                        println!("[quota-ui] Manual refresh triggered via right-click");
                        trigger_background_fetch(tx.clone());
                    }
                }
                x11rb::protocol::Event::ButtonRelease(ev) => {
                    if ev.detail == 1 && is_dragging {
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

    save_config(&cfg);
    println!("[quota-ui] Exited cleanly.");
    Ok(())
}
