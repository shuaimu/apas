//! Regenerate the cross-emulator fixture used by the web regression test:
//! cargo run -p apas --example terminal_checkpoint_fixture > packages/web/src/lib/terminalCheckpoints.fixture.json
#[path = "../src/terminal_screen.rs"]
mod terminal_screen;
use base64::Engine;
use serde_json::json;
use terminal_screen::TerminalScreen;

fn main() {
    if std::env::var_os("APAS_TERMINAL_BENCH").is_some() {
        let mut screen = TerminalScreen::new(80, 24);
        let start = std::time::Instant::now();
        for _ in 0..128 {
            screen.process(&vec![b'x'; 8192]);
        }
        eprintln!(
            "1 MiB parsed in {:?}, checkpoint available: {}",
            start.elapsed(),
            screen.checkpoint(1).is_some()
        );
        return;
    }
    let mut fixtures = Vec::new();
    let cases = [
        ("combining", "e\u{301} 👩‍💻 🇺🇸", "more"),
        ("charset", "\x1b(0lqk", "qjx\x1b(Bdone"),
        ("save-wrap", "\x1b[1;80HZ\x1b7\r\nelsewhere", "\x1b8Q"),
        (
            "alt47",
            "\x1b[2;3H\x1b7\x1b[5;8Hnormal\x1b[?47hALT",
            "\x1b[?1049lQ",
        ),
        ("csi-c0", "normal\x1b[3;\n", "4Hnext"),
        ("partial-osc", "normal\x1b]0;hello", "world\x07next"),
        (
            "normal",
            "hello\r\n\x1b[31mred\x1b[0m 日本語 🤖",
            "\r\nnext",
        ),
        (
            "alternate-mouse",
            "normal\x1b[?1049h\x1b[?1002h\x1b[?1006h\x1b[?1h\x1b[?2004h\x1b=\x1b[3;9H\x1b[31mHELLO",
            "\x1b[?1049l\r\nback",
        ),
        (
            "region",
            "\x1b[2;8r\x1b[?6h\x1b[3;4Hregion\x1b[?25l",
            "\r\n\r\n\r\nmore",
        ),
        ("wrap", "\x1b[2;79HABC", "DEF\r\nnext"),
        ("saved-cursor", "\x1b[4;6H\x1b7\x1b[9;9Haway", "\x1b8back"),
        ("split-csi", "normal\x1b[?1049h\x1b[?1006", "h\x1b[3;4Hnext"),
        ("split-sgr", "normal\x1b[38;2;15;", "42;55mcolor"),
    ];
    for (name, prefix, tail) in cases {
        let mut screen = TerminalScreen::new(80, 24);
        screen.process(prefix.as_bytes());
        fixtures.push(json!({"name": name, "prefix": prefix, "repeat": 1, "tail": tail, "checkpoint": screen.checkpoint(10)}));
    }
    let prefix = "line with scrolling text\r\n";
    let mut screen = TerminalScreen::new(80, 24);
    for _ in 0..40000 {
        screen.process(prefix.as_bytes());
    }
    screen.process(b"\x1b[?1049h\x1b[?1002h\x1b[?1006h\x1b[2;3Hlast frame");
    fixtures.push(json!({"name": "overflow", "prefix": prefix, "repeat": 40000, "suffix": "\x1b[?1049h\x1b[?1002h\x1b[?1006h\x1b[2;3Hlast frame", "tail": "\x1b[?1049l", "checkpoint": screen.checkpoint(20)}));
    let mut screen = TerminalScreen::new(80, 24);
    screen.process(&[b'A', 0xf0, 0x9f]);
    fixtures.push(json!({"name": "split-utf8", "prefixBase64": "QfCf", "tailBase64": base64::engine::general_purpose::STANDARD.encode([0xa4, 0x96]), "checkpoint": screen.checkpoint(2)}));
    println!("{}", serde_json::to_string_pretty(&fixtures).unwrap());
}
