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

struct BtnArea {
    btn: ActionBtn,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

struct LogoArea {
    logo: HoveredLogo,
    name: &'static str,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

/// Dynamic dimensions:
/// When NOT hovered: Ultra-compact, only Logo + Ring Meters (Horizontal: 208px, Vertical: 168px)
/// When hovered: Extends to reveal control buttons (Horizontal: +58px = 266px, Vertical: +48px = 216px)
fn get_dimensions(cfg: &AppConfig, is_hovered: bool) -> (usize, usize) {
    let (base_w, base_h) = match (cfg.orientation, is_hovered) {
        (Orientation::Horizontal, false) => (208, 38),
        (Orientation::Horizontal, true) => (266, 38),
        (Orientation::Vertical, false) => (44, 168),
        (Orientation::Vertical, true) => (44, 216),
    };
    let multiplier = match cfg.scale {
        ScaleSize::Small => 0.88,
        ScaleSize::Normal => 1.0,
        ScaleSize::Large => 1.22,
    };
    (
        ((base_w as f64) * multiplier).round() as usize,
        ((base_h as f64) * multiplier).round() as usize,
    )
}

fn get_button_areas(w: usize, h: usize, orientation: Orientation) -> Vec<BtnArea> {
    let mut areas = Vec::new();
    match orientation {
        Orientation::Horizontal => {
            let btn_size = 10;
            let gap = 4;
            let total_w = 4 * btn_size + 3 * gap;
            let start_x = w as i32 - total_w - 9;
            let start_y = (h as i32 - btn_size) / 2;

            let btns = [
                ActionBtn::Orientation,
                ActionBtn::Resize,
                ActionBtn::Refresh,
                ActionBtn::Close,
            ];
            for (i, &b) in btns.iter().enumerate() {
                areas.push(BtnArea {
                    btn: b,
                    x: start_x + (i as i32) * (btn_size + gap),
                    y: start_y,
                    w: btn_size,
                    h: btn_size,
                });
            }
        }
        Orientation::Vertical => {
            let btn_size = 10;
            let gap = 4;
            let total_h = 4 * btn_size + 3 * gap;
            let start_x = (w as i32 - btn_size) / 2;
            let start_y = h as i32 - total_h - 8;

            let btns = [
                ActionBtn::Orientation,
                ActionBtn::Resize,
                ActionBtn::Refresh,
                ActionBtn::Close,
            ];
            for (i, &b) in btns.iter().enumerate() {
                areas.push(BtnArea {
                    btn: b,
                    x: start_x,
                    y: start_y + (i as i32) * (btn_size + gap),
                    w: btn_size,
                    h: btn_size,
                });
            }
        }
    }
    areas
}

fn get_logo_areas(h: usize, orientation: Orientation) -> Vec<LogoArea> {
    match orientation {
        Orientation::Horizontal => vec![
            LogoArea {
                logo: HoveredLogo::Claude,
                name: "Claude",
                x: 16,
                y: 6,
                w: 18,
                h: (h as i32) - 12,
            },
            LogoArea {
                logo: HoveredLogo::Codex,
                name: "Codex",
                x: 81,
                y: 6,
                w: 18,
                h: (h as i32) - 12,
            },
            LogoArea {
                logo: HoveredLogo::Gemini,
                name: "Gemini",
                x: 146,
                y: 6,
                w: 18,
                h: (h as i32) - 12,
            },
        ],
        Orientation::Vertical => {
            vec![
                LogoArea {
                    logo: HoveredLogo::Claude,
                    name: "Claude",
                    x: 6,
                    y: 18,
                    w: 32,
                    h: 18,
                },
                LogoArea {
                    logo: HoveredLogo::Codex,
                    name: "Codex",
                    x: 6,
                    y: 71,
                    w: 32,
                    h: 18,
                },
                LogoArea {
                    logo: HoveredLogo::Gemini,
                    name: "Gemini",
                    x: 6,
                    y: 124,
                    w: 32,
                    h: 18,
                },
            ]
        }
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
    let (w, h) = get_dimensions(cfg, is_hovered);
    fb.resize(w, h);
    fb.clear(Color::rgb(12, 16, 23));

    let btn_areas = get_button_areas(w, h, cfg.orientation);
    let logo_areas = get_logo_areas(h, cfg.orientation);

    match cfg.orientation {
        Orientation::Horizontal => {
            // Drag handle on far left
            fb.draw_drag_grip(7, (h as i32 - 10) / 2, false);

            // 1. Claude
            fb.draw_claude_logo(20, (h as i32 - 10) / 2);
            fb.draw_ring_gauge(
                53,
                (h as i32) / 2,
                12.0,
                2.5,
                data.claude_5h,
                Color::rgb(192, 132, 252),
            );

            // Divider 1
            fb.draw_divider_v(76);

            // 2. Codex
            fb.draw_codex_logo(85, (h as i32 - 10) / 2);
            fb.draw_ring_gauge(
                118,
                (h as i32) / 2,
                12.0,
                2.5,
                data.codex_5h,
                Color::rgb(52, 211, 153),
            );

            // Divider 2
            fb.draw_divider_v(141);

            // 3. Gemini
            fb.draw_gemini_logo(150, (h as i32 - 10) / 2);
            fb.draw_ring_gauge(
                183,
                (h as i32) / 2,
                12.0,
                2.5,
                data.gemini,
                Color::rgb(96, 165, 250),
            );

            // Action Buttons: Only rendered when hovered
            if is_hovered {
                fb.draw_divider_v(206);
                for area in btn_areas {
                    let is_target = hovered_btn == Some(area.btn);
                    match area.btn {
                        ActionBtn::Orientation => {
                            fb.draw_icon_orientation(area.x, area.y + 1, is_target);
                        }
                        ActionBtn::Resize => {
                            fb.draw_icon_resize(area.x, area.y + 1, is_target);
                        }
                        ActionBtn::Refresh => {
                            fb.draw_icon_refresh(area.x, area.y + 1, is_target);
                        }
                        ActionBtn::Close => {
                            fb.draw_icon_close(area.x + 1, area.y + 1, is_target);
                        }
                    }
                }
            }

            // Draw Logo Tooltip if hovered
            if let Some(target) = hovered_logo {
                if let Some(area) = logo_areas.iter().find(|a| a.logo == target) {
                    fb.draw_tooltip(area.x + area.w / 2, (h as i32) / 2 - 4, area.name);
                }
            }
        }
        Orientation::Vertical => {
            let center_x = (w as i32) / 2;

            // Drag handle at top
            fb.draw_drag_grip((w as i32 - 10) / 2, 7, true);

            // 1. Claude
            fb.draw_claude_logo((w as i32 - 10) / 2, 22);
            fb.draw_ring_gauge(
                center_x,
                48,
                12.0,
                2.5,
                data.claude_5h,
                Color::rgb(192, 132, 252),
            );

            fb.draw_divider_h(67);

            // 2. Codex
            fb.draw_codex_logo((w as i32 - 10) / 2, 75);
            fb.draw_ring_gauge(
                center_x,
                101,
                12.0,
                2.5,
                data.codex_5h,
                Color::rgb(52, 211, 153),
            );

            fb.draw_divider_h(120);

            // 3. Gemini
            fb.draw_gemini_logo((w as i32 - 10) / 2, 128);
            fb.draw_ring_gauge(
                center_x,
                154,
                12.0,
                2.5,
                data.gemini,
                Color::rgb(96, 165, 250),
            );

            // Action Buttons: Only rendered when hovered
            if is_hovered {
                fb.draw_divider_h(172);
                for area in btn_areas {
                    let is_target = hovered_btn == Some(area.btn);
                    match area.btn {
                        ActionBtn::Orientation => {
                            fb.draw_icon_orientation(area.x, area.y + 1, is_target);
                        }
                        ActionBtn::Resize => {
                            fb.draw_icon_resize(area.x, area.y + 1, is_target);
                        }
                        ActionBtn::Refresh => {
                            fb.draw_icon_refresh(area.x, area.y + 1, is_target);
                        }
                        ActionBtn::Close => {
                            fb.draw_icon_close(area.x + 1, area.y + 1, is_target);
                        }
                    }
                }
            }

            // Draw Logo Tooltip if hovered
            if let Some(target) = hovered_logo {
                if let Some(area) = logo_areas.iter().find(|a| a.logo == target) {
                    fb.draw_tooltip(center_x, area.y - 2, area.name);
                }
            }
        }
    }

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

    let (w, h) = get_dimensions(&cfg, is_hovered);

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
        w as u16,
        h as u16,
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

    let mut fb = Framebuffer::new(w, h);
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
                        let (new_w, new_h) = get_dimensions(&cfg, is_hovered);
                        conn.configure_window(
                            win,
                            &ConfigureWindowAux::new()
                                .width(new_w as u32)
                                .height(new_h as u32),
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
                        let (new_w, new_h) = get_dimensions(&cfg, is_hovered);
                        conn.configure_window(
                            win,
                            &ConfigureWindowAux::new()
                                .width(new_w as u32)
                                .height(new_h as u32),
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
                        let (curr_w, curr_h) = get_dimensions(&cfg, is_hovered);
                        let btn_areas = get_button_areas(curr_w, curr_h, cfg.orientation);
                        let logo_areas = get_logo_areas(curr_h, cfg.orientation);
                        let mx = ev.event_x as i32;
                        let my = ev.event_y as i32;

                        let mut target_btn = None;
                        for a in btn_areas {
                            if mx >= a.x && mx < a.x + a.w && my >= a.y && my < a.y + a.h {
                                target_btn = Some(a.btn);
                                break;
                            }
                        }

                        let mut target_logo = None;
                        for a in logo_areas {
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
                                let (new_w, new_h) = get_dimensions(&cfg, is_hovered);
                                conn.configure_window(
                                    win,
                                    &ConfigureWindowAux::new()
                                        .width(new_w as u32)
                                        .height(new_h as u32),
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
                                    let (new_w, new_h) = get_dimensions(&cfg, is_hovered);
                                    conn.configure_window(
                                        win,
                                        &ConfigureWindowAux::new()
                                            .width(new_w as u32)
                                            .height(new_h as u32),
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
                                    let (new_w, new_h) = get_dimensions(&cfg, is_hovered);
                                    conn.configure_window(
                                        win,
                                        &ConfigureWindowAux::new()
                                            .width(new_w as u32)
                                            .height(new_h as u32),
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
                        let (new_w, new_h) = get_dimensions(&cfg, is_hovered);
                        conn.configure_window(
                            win,
                            &ConfigureWindowAux::new()
                                .width(new_w as u32)
                                .height(new_h as u32),
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
