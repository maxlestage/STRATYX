//! Test d'intégration X11 sur un vrai serveur d'affichage (Xvfb en CI) :
//! `xvfb-run -a cargo test -p stratyx-capture`. Ignoré si `DISPLAY` n'est pas défini.
#![cfg(target_os = "linux")]

use std::time::Duration;

use stratyx_capture::{CaptureTarget, Rect, ScreenCapturer, X11Capturer};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, CreateWindowAux, PropMode, WindowClass};
use x11rb::wrapper::ConnectionExt as _;

#[test]
fn captures_window_and_screen_region() {
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("DISPLAY absent : test X11 ignoré (lancer avec xvfb-run)");
        return;
    }
    let (conn, screen_num) = x11rb::connect(None).expect("connexion X11");
    let screen = &conn.setup().roots[screen_num];
    // Fenêtre 200×120 orange (#FF8040) contenant un carré bleu (#2050C0) en (50, 30).
    let win = conn.generate_id().unwrap();
    conn.create_window(
        screen.root_depth,
        win,
        screen.root,
        40,
        60,
        200,
        120,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new()
            .background_pixel(0x00FF_8040)
            .override_redirect(1),
    )
    .unwrap();
    let child = conn.generate_id().unwrap();
    conn.create_window(
        screen.root_depth,
        child,
        win,
        50,
        30,
        20,
        10,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new().background_pixel(0x0020_50C0),
    )
    .unwrap();
    conn.change_property8(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        b"Table Stratyx",
    )
    .unwrap();
    conn.map_window(child).unwrap();
    conn.map_window(win).unwrap();
    conn.flush().unwrap();
    conn.sync().unwrap();
    std::thread::sleep(Duration::from_millis(300));

    let mut cap = X11Capturer::connect().unwrap();
    let targets = cap.list_targets().unwrap();
    let info = targets
        .iter()
        .find(|t| t.title == "Table Stratyx")
        .expect("fenêtre listée");
    assert_eq!((info.width, info.height), (200, 120));

    let target = CaptureTarget::Window { id: u64::from(win) };
    let frame = cap.grab(&target, None).unwrap();
    assert_eq!((frame.width, frame.height), (200, 120));
    assert_eq!(frame.pixel(5, 5), Some([0xFF, 0x80, 0x40, 255]));
    assert_eq!(frame.pixel(55, 35), Some([0x20, 0x50, 0xC0, 255]));

    // Zone de la fenêtre.
    let zone = cap
        .grab(
            &target,
            Some(Rect {
                x: 50,
                y: 30,
                width: 20,
                height: 10,
            }),
        )
        .unwrap();
    assert!(
        zone.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == [0x20, 0x50, 0xC0, 255])
    );

    // Écran entier : la fenêtre est à (40, 60).
    let full = cap.grab(&CaptureTarget::Screen { index: 0 }, None).unwrap();
    assert_eq!(full.pixel(40 + 55, 60 + 35), Some([0x20, 0x50, 0xC0, 255]));

    // Zone hors cible refusée.
    assert!(
        cap.grab(
            &target,
            Some(Rect {
                x: 190,
                y: 0,
                width: 20,
                height: 5
            })
        )
        .is_err()
    );
}
