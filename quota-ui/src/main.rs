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
                        BtnArea { btn: ActionBtn::Orientation, x: 155, y: 11, w: 9, h: 9 },
                        BtnArea { btn: ActionBtn::Resize, x: 166, y: 11, w: 9, h: 9 },
                        BtnArea { btn: ActionBtn::Refresh, x: 177, y: 11, w: 9, h: 9 },
                        BtnArea { btn: ActionBtn::Close, x: 188, y: 11, w: 9, h: 9 },
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
                        LogoArea { logo: HoveredLogo::Claude, name: "Claude", x: 14, y: 3, w: 14, h: 24, tooltip_anchor: (21, 8) },
                        LogoArea { logo: HoveredLogo::Codex, name: "Codex", x: 61, y: 3, w: 14, h: 24, tooltip_anchor: (68, 8) },
                        LogoArea { logo: HoveredLogo::Gemini, name: "Gemini", x: 108, y: 3, w: 14, h: 24, tooltip_anchor: (115, 8) },
                    ],
                }
            }
            ScaleSize::Normal => {
                let win_h = 40;
                let (win_w, btn_div, buttons) = if is_hovered {
                    let btns = vec![
                        BtnArea { btn: ActionBtn::Orientation, x: 208, y: 15, w: 12, h: 12 },
                        BtnArea { btn: ActionBtn::Resize, x: 222, y: 15, w: 12, h: 12 },
                        BtnArea { btn: ActionBtn::Refresh, x: 236, y: 15, w: 12, h: 12 },
                        BtnArea { btn: ActionBtn::Close, x: 250, y: 15, w: 12, h: 12 },
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
                        LogoArea { logo: HoveredLogo::Claude, name: "Claude", x: 18, y: 4, w: 18, h: 32, tooltip_anchor: (27, 10) },
                        LogoArea { logo: HoveredLogo::Codex, name: "Codex", x: 82, y: 4, w: 18, h: 32, tooltip_anchor: (91, 10) },
                        LogoArea { logo: HoveredLogo::Gemini, name: "Gemini", x: 146, y: 4, w: 18, h: 32, tooltip_anchor: (155, 10) },
                    ],
                }
            }
            ScaleSize::Large => {
                let win_h = 58;
                let (win_w, btn_div, buttons) = if is_hovered {
                    let btns = vec![
                        BtnArea { btn: ActionBtn::Orientation, x: 295, y: 22, w: 16, h: 16 },
                        BtnArea { btn: ActionBtn::Resize, x: 315, y: 22, w: 16, h: 16 },
                        BtnArea { btn: ActionBtn::Refresh, x: 335, y: 22, w: 16, h: 16 },
                        BtnArea { btn: ActionBtn::Close, x: 355, y: 22, w: 16, h: 16 },
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
                        LogoArea { logo: HoveredLogo::Claude, name: "Claude", x: 25, y: 6, w: 26, h: 46, tooltip_anchor: (38, 14) },
                        LogoArea { logo: HoveredLogo::Codex, name: "Codex", x: 116, y: 6, w: 26, h: 46, tooltip_anchor: (129, 14) },
                        LogoArea { logo: HoveredLogo::Gemini, name: "Gemini", x: 207, y: 6, w: 26, h: 46, tooltip_anchor: (220, 14) },
                    ],
                }
            }
        },
        Orientation::Vertical => match cfg.scale {
            ScaleSize::Small => {
                let win_w = 36;
                let (win_h, btn_div, buttons) = if is_hovered {
                    let btns = vec![
                        BtnArea { btn: ActionBtn::Orientation, x: 14, y: 135, w: 9, h: 9 },
                        BtnArea { btn: ActionBtn::Resize, x: 14, y: 146, w: 9, h: 9 },
                        BtnArea { btn: ActionBtn::Refresh, x: 14, y: 157, w: 9, h: 9 },
                        BtnArea { btn: ActionBtn::Close, x: 14, y: 168, w: 9, h: 9 },
                    ];
                    (182, Some(129), btns)
                } else {
                    (130, None, Vec::new())
                };
                LayoutMetrics {
                    win_w,
                    win_h,
                    grip_pos: (13, 6),
                    claude_logo: (13, 14),
                    claude_gauge: (18, 35),
                    codex_logo: (13, 54),
                    codex_gauge: (18, 75),
                    gemini_logo: (13, 94),
                    gemini_gauge: (18, 115),
                    dividers: vec![49, 89],
                    btn_divider: btn_div,
                    buttons,
                    logos: vec![
                        LogoArea { logo: HoveredLogo::Claude, name: "Claude", x: 4, y: 12, w: 28, h: 14, tooltip_anchor: (18, 11) },
                        LogoArea { logo: HoveredLogo::Codex, name: "Codex", x: 4, y: 52, w: 28, h: 14, tooltip_anchor: (18, 51) },
                        LogoArea { logo: HoveredLogo::Gemini, name: "Gemini", x: 4, y: 92, w: 28, h: 14, tooltip_anchor: (18, 91) },
                    ],
                }
            }
            ScaleSize::Normal => {
                let win_w = 46;
                let (win_h, btn_div, buttons) = if is_hovered {
                    let btns = vec![
                        BtnArea { btn: ActionBtn::Orientation, x: 17, y: 176, w: 12, h: 12 },
                        BtnArea { btn: ActionBtn::Resize, x: 17, y: 190, w: 12, h: 12 },
                        BtnArea { btn: ActionBtn::Refresh, x: 17, y: 204, w: 12, h: 12 },
                        BtnArea { btn: ActionBtn::Close, x: 17, y: 218, w: 12, h: 12 },
                    ];
                    (236, Some(169), btns)
                } else {
                    (170, None, Vec::new())
                };
                LayoutMetrics {
                    win_w,
                    win_h,
                    grip_pos: (18, 7),
                    claude_logo: (16, 18),
                    claude_gauge: (23, 46),
                    codex_logo: (16, 70),
                    codex_gauge: (23, 98),
                    gemini_logo: (16, 122),
                    gemini_gauge: (23, 150),
                    dividers: vec![64, 116],
                    btn_divider: btn_div,
                    buttons,
                    logos: vec![
                        LogoArea { logo: HoveredLogo::Claude, name: "Claude", x: 5, y: 16, w: 36, h: 17, tooltip_anchor: (23, 15) },
                        LogoArea { logo: HoveredLogo::Codex, name: "Codex", x: 5, y: 68, w: 36, h: 17, tooltip_anchor: (23, 67) },
                        LogoArea { logo: HoveredLogo::Gemini, name: "Gemini", x: 5, y: 120, w: 36, h: 17, tooltip_anchor: (23, 119) },
                    ],
                }
            }
            ScaleSize::Large => {
                let win_w = 64;
                let (win_h, btn_div, buttons) = if is_hovered {
                    let btns = vec![
                        BtnArea { btn: ActionBtn::Orientation, x: 24, y: 252, w: 16, h: 16 },
                        BtnArea { btn: ActionBtn::Resize, x: 24, y: 272, w: 16, h: 16 },
                        BtnArea { btn: ActionBtn::Refresh, x: 24, y: 292, w: 16, h: 16 },
                        BtnArea { btn: ActionBtn::Close, x: 24, y: 312, w: 16, h: 16 },
                    ];
                    (336, Some(243), btns)
                } else {
                    (244, None, Vec::new())
                };
                LayoutMetrics {
                    win_w,
                    win_h,
                    grip_pos: (22, 9),
                    claude_logo: (22, 25),
                    claude_gauge: (32, 66),
                    codex_logo: (22, 100),
                    codex_gauge: (32, 141),
                    gemini_logo: (22, 175),
                    gemini_gauge: (32, 216),
                    dividers: vec![92, 167],
                    btn_divider: btn_div,
                    buttons,
                    logos: vec![
                        LogoArea { logo: HoveredLogo::Claude, name: "Claude", x: 7, y: 22, w: 50, h: 26, tooltip_anchor: (32, 20) },
                        LogoArea { logo: HoveredLogo::Codex, name: "Codex", x: 7, y: 97, w: 50, h: 26, tooltip_anchor: (32, 95) },
                        LogoArea { logo: HoveredLogo::Gemini, name: "Gemini", x: 7, y: 172, w: 50, h: 26, tooltip_anchor: (32, 170) },
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
            match area.btn {
                ActionBtn::Orientation => {
                    fb.draw_icon_orientation(area.x, area.y, is_target, cfg.scale);
                }
                ActionBtn::Resize => {
                    fb.draw_icon_resize(area.x, area.y, is_target, cfg.scale);
                }
                ActionBtn::Refresh => {
                    fb.draw_icon_refresh(area.x, area.y, is_target, cfg.scale);
                }
                ActionBtn::Close => {
                    fb.draw_icon_close(area.x, area.y, is_target, cfg.scale);
                }
            }
        }
    }

    // 7. Logo Tooltip (when logo hovered)
    if let Some(target) = hovered_logo {
        if let Some(area) = layout.logos.iter().find(|a| a.logo == target) {
            fb.draw_tooltip(area.tooltip_anchor.0, area.tooltip_anchor.1, area.name);
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

        // 3. Process X11 Events
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
                x11rb::protocol::Event::EnterNotify(_) => {
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
                x11rb::protocol::Event::LeaveNotify(_) => {
                    if !is_dragging && is_hovered {
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
                            hovered_btn,
                            hovered_logo,
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
                            || !is_hovered
                        {
                            let need_resize = !is_hovered;
                            is_hovered = true;
                            hovered_btn = target_btn;
                            hovered_logo = target_logo;

                            if need_resize {
                                let new_layout = compute_layout(&cfg, is_hovered);
                                conn.configure_window(
                                    win,
                                    &ConfigureWindowAux::new()
                                        .width(new_layout.win_w as u32)
                                        .height(new_layout.win_h as u32),
                                )?;
                                conn.flush()?;
                            }

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
