mod config;
mod fetcher;
mod font;
mod http;
mod json;
mod render;

use config::{load_position, save_position};
use fetcher::{fetch_all, QuotaSummary};
use render::{Color, Framebuffer, HEIGHT, WIDTH};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

fn draw_widget(fb: &mut Framebuffer, data: &QuotaSummary) {
    // 1. Deep translucent dark background (#0c1017)
    fb.clear(Color::rgb(12, 16, 23));

    // 2. Drag handle on far left
    fb.draw_drag_grip(7, 14);

    // 3. Section 1: Claude (x: 20..78)
    fb.draw_claude_logo(20, 13);
    fb.draw_ring_gauge(
        54,
        18,
        12.0,
        2.5,
        data.claude_5h,
        Color::rgb(192, 132, 252),
    );

    // Divider 1
    fb.draw_divider(79);

    // 4. Section 2: Codex (x: 88..146)
    fb.draw_codex_logo(88, 13);
    fb.draw_ring_gauge(
        122,
        18,
        12.0,
        2.5,
        data.codex_5h,
        Color::rgb(52, 211, 153),
    );

    // Divider 2
    fb.draw_divider(147);

    // 5. Section 3: Gemini (x: 156..214)
    fb.draw_gemini_logo(156, 13);
    fb.draw_ring_gauge(
        190,
        18,
        12.0,
        2.5,
        data.gemini,
        Color::rgb(96, 165, 250),
    );

    // 6. Subtle border glow
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

    // Load initial position or center horizontally near top
    let pos = load_position().unwrap_or_else(|| {
        let x = (screen.width_in_pixels as i32 - WIDTH as i32) / 2;
        let y = 32;
        config::Position { x, y }
    });

    let win = conn.generate_id()?;
    let win_aux = CreateWindowAux::new()
        .background_pixel(Color::rgb(12, 16, 23).to_x11_pixel())
        .override_redirect(1) // Borderless floating HUD window
        .event_mask(
            EventMask::EXPOSURE
                | EventMask::BUTTON_PRESS
                | EventMask::BUTTON_RELEASE
                | EventMask::POINTER_MOTION
                | EventMask::KEY_PRESS,
        );

    conn.create_window(
        24,
        win,
        root,
        pos.x as i16,
        pos.y as i16,
        WIDTH as u16,
        HEIGHT as u16,
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

    // Set _NET_WM_WINDOW_TYPE to UTILITY or DOCK
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

    let mut fb = Framebuffer::new(WIDTH, HEIGHT);
    let current_data = Arc::new(Mutex::new(QuotaSummary::default()));

    // Channel for background quota updates
    let (tx, rx): (Sender<QuotaSummary>, Receiver<QuotaSummary>) = channel();
    trigger_background_fetch(tx.clone());

    // Dragging state
    let mut is_dragging = false;
    let mut drag_start_cursor_x = 0;
    let mut drag_start_cursor_y = 0;
    let mut drag_start_win_x = pos.x;
    let mut drag_start_win_y = pos.y;
    let mut current_win_x = pos.x;
    let mut current_win_y = pos.y;

    let mut last_poll = Instant::now();
    let poll_interval = Duration::from_secs(300); // 5 minutes

    // Initial render
    {
        let data = current_data.lock().unwrap();
        draw_widget(&mut fb, &data);
    }
    upload_frame(&conn, win, gc, &fb)?;

    'main_loop: loop {
        // 1. Check if background fetcher returned fresh quota
        if let Ok(fresh) = rx.try_recv() {
            {
                let mut data = current_data.lock().unwrap();
                *data = fresh;
                draw_widget(&mut fb, &data);
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
                    draw_widget(&mut fb, &data);
                    upload_frame(&conn, win, gc, &fb)?;
                }
                x11rb::protocol::Event::ButtonPress(ev) => {
                    if ev.detail == 1 {
                        // Left click -> Start Drag
                        is_dragging = true;
                        drag_start_cursor_x = ev.root_x as i32;
                        drag_start_cursor_y = ev.root_y as i32;
                        drag_start_win_x = current_win_x;
                        drag_start_win_y = current_win_y;
                    } else if ev.detail == 3 {
                        // Right click -> Manual refresh
                        println!("[quota-ui] Manual refresh triggered");
                        trigger_background_fetch(tx.clone());
                    }
                }
                x11rb::protocol::Event::MotionNotify(ev) => {
                    if is_dragging {
                        let dx = ev.root_x as i32 - drag_start_cursor_x;
                        let dy = ev.root_y as i32 - drag_start_cursor_y;
                        current_win_x = drag_start_win_x + dx;
                        current_win_y = drag_start_win_y + dy;

                        conn.configure_window(
                            win,
                            &ConfigureWindowAux::new()
                                .x(current_win_x)
                                .y(current_win_y),
                        )?;
                        conn.flush()?;
                    }
                }
                x11rb::protocol::Event::ButtonRelease(ev) => {
                    if ev.detail == 1 && is_dragging {
                        is_dragging = false;
                        save_position(current_win_x, current_win_y);
                    }
                }
                x11rb::protocol::Event::KeyPress(ev) => {
                    // Esc key (keycode 9) to close
                    if ev.detail == 9 {
                        break 'main_loop;
                    }
                    // 'r' key (keycode 27) to refresh
                    if ev.detail == 27 {
                        trigger_background_fetch(tx.clone());
                    }
                }
                _ => {}
            }
        }

        thread::sleep(Duration::from_millis(16)); // ~60 FPS responsiveness for smooth dragging
    }

    save_position(current_win_x, current_win_y);
    println!("[quota-ui] Exited cleanly.");
    Ok(())
}
