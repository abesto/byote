/*** includes ***/

#[macro_use]
extern crate lazy_static;

#[macro_use]
extern crate simple_error;

use libc::{atexit, ioctl, winsize, TIOCGWINSZ};
use nix::Error;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::process::exit;
use termios::{
    tcsetattr, Termios, BRKINT, CS8, ECHO, ICANON, ICRNL, IEXTEN, INPCK, ISIG, ISTRIP, IXON, OPOST,
    TCSAFLUSH, VMIN, VTIME,
};

/*** defines ***/
type Result<T> = std::result::Result<T, Box<std::error::Error>>;

fn ctrl_key(k: u8) -> u8 {
    k & 0x1f
}

#[derive(Ord, PartialOrd, Eq, PartialEq)]
enum EditorKey {
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,

    Home,
    End,
    PageUp,
    PageDown,

    Escape,

    Char(u8),
}

const BYOTE_VERSION: Option<&'static str> = option_env!("CARGO_PKG_VERSION");

/*** data ***/

struct EditorConfig {
    screenrows: usize,
    screencols: usize,
    cx: usize,
    cy: usize,
}

impl EditorConfig {
    fn from_env() -> Result<EditorConfig> {
        let (rows, cols) = get_window_size()?;
        Ok(EditorConfig {
            screenrows: rows,
            screencols: cols,
            cx: 0,
            cy: 0,
        })
    }
}

lazy_static! {
    static ref STDIN_RAWFD: RawFd = std::io::stdin().as_raw_fd();
    static ref STDOUT_RAWFD: RawFd = std::io::stdout().as_raw_fd();
    static ref ORIG_TERMIOS: Termios = Termios::from_fd(*STDIN_RAWFD)
        .map_err(|_| die("EditorConfig::from_fb/orig_termios"))
        .unwrap();
}

/*** terminal ***/

#[allow(clippy::print_with_newline)]
fn die(s: &str) {
    print!("\x1b[2J\x1b[H{}: {}\r\n", s, Error::last().to_string());
    flush_stdout();
    exit(1);
}

extern "C" fn disable_raw_mode() {
    if tcsetattr(*STDIN_RAWFD, TCSAFLUSH, &*ORIG_TERMIOS).is_err() {
        die("disable_raw_mode/tcsetattr");
    }
}

fn enable_raw_mode() {
    unsafe {
        atexit(disable_raw_mode);
    };

    let mut raw: Termios = *ORIG_TERMIOS;
    raw.c_iflag &= !(BRKINT | ICRNL | INPCK | ISTRIP | IXON);
    raw.c_oflag &= !(OPOST);
    raw.c_cflag |= CS8;
    raw.c_lflag &= !(ECHO | ICANON | IEXTEN | ISIG);
    raw.c_cc[VMIN] = 0;
    raw.c_cc[VTIME] = 1;
    if tcsetattr(*STDIN_RAWFD, TCSAFLUSH, &raw).is_err() {
        die("enable_raw_mode/tcsetattr");
    }
}

fn editor_read_key() -> EditorKey {
    let mut buffer: [u8; 1] = [0];
    loop {
        match std::io::stdin().read(&mut buffer) {
            Err(ref e) if e.kind() != ErrorKind::Interrupted => die("editor_read_key/read"),
            Err(e) => die(&format!("editor_read_key: {}", e)),
            Ok(1) => {
                let c = buffer[0];
                if c == b'\x1b' {
                    let mut buffer = [0_u8; 3];

                    // I think we're potentially losing a character or two here? If there's a char
                    // after the escape, but not matching what we expect?

                    let result = std::io::stdin().read(&mut buffer);
                    if result.is_err() {
                        return EditorKey::Escape;
                    }

                    let seq = &buffer[..result.unwrap()];

                    return match std::str::from_utf8(seq).unwrap() {
                        "[A" => EditorKey::ArrowUp,
                        "[B" => EditorKey::ArrowDown,
                        "[C" => EditorKey::ArrowRight,
                        "[D" => EditorKey::ArrowLeft,

                        "[5~" => EditorKey::PageUp,
                        "[6~" => EditorKey::PageDown,
                        "[1~" | "[7~" | "[H" | "OH" => EditorKey::Home,
                        "[4~" | "[8~" | "[F" | "OF" => EditorKey::End,

                        _ => EditorKey::Escape,
                    };
                } else {
                    return EditorKey::Char(c);
                }
            }
            Ok(0) => (),
            Ok(n) => die(&format!(
                "editor_read_key read unexpected number of chars: {}",
                n
            )),
        }
    }
}

fn get_cursor_position() -> Result<(usize, usize)> {
    std::io::stdout().write_all(b"\x1b[6n\r\n")?;
    flush_stdout();

    let mut buffer = [0_u8; 32];
    let mut i = 0;
    while i < buffer.len() {
        if std::io::stdin().read_exact(&mut buffer[i..=i]).is_err() {
            break;
        }
        if buffer[i] == b'R' {
            break;
        }
        i += 1;
    }

    let output = std::str::from_utf8(&buffer[0..i]).unwrap();
    if &output[0..=1] != "\x1b[" {
        bail!("get_cursor_position/invalid-response");
    }

    let rows_and_cols: Vec<&str> = output[2..i].split(';').collect();
    if rows_and_cols.len() != 2 {
        bail!("get_cursor_position/split/len");
    }

    Ok((
        rows_and_cols[0].parse::<usize>()?,
        rows_and_cols[1].parse::<usize>()?,
    ))
}

fn get_window_size() -> Result<(usize, usize)> {
    let mut ws: winsize = unsafe { std::mem::zeroed() };
    let result = unsafe { ioctl(*STDOUT_RAWFD, TIOCGWINSZ, &mut ws) };
    if result == -1 || ws.ws_col == 0 {
        let result = std::io::stdout().write(b"\x1b[999C\x1b[999B");
        flush_stdout();
        match result {
            Ok(12) => get_cursor_position(),
            _ => bail!("failed to determine window size"),
        }
    } else {
        Ok((ws.ws_row as usize, ws.ws_col as usize))
    }
}

/*** output ***/
fn flush_stdout() {
    std::io::stdout()
        .flush()
        .map_err(|e| die(&format!("flush_stdout: {}", e)))
        .unwrap();
}

fn editor_refresh_screen(e: &EditorConfig) {
    let mut buffer = String::new();

    buffer += "\x1b[?25l";
    buffer += "\x1b[H";

    editor_draw_rows(e, &mut buffer);

    buffer += &format!("\x1b[{};{}H", e.cy + 1, e.cx + 1);

    buffer += "\x1b[?25h";

    print!("{}", buffer);

    flush_stdout();
}

#[allow(clippy::print_with_newline)]
fn editor_draw_rows(e: &EditorConfig, buffer: &mut String) {
    for y in 0..e.screenrows {
        if (y == e.screenrows / 3) {
            let mut msg = format!("BYOTE -- version {}", BYOTE_VERSION.unwrap_or("unknown"));
            msg.truncate(e.screencols);
            let mut padding = (e.screencols - msg.len()) / 2;
            if padding > 0 {
                *buffer += "~";
                padding -= 1;
            }
            while padding > 0 {
                *buffer += " ";
                padding -= 1;
            }

            *buffer += &msg;
        } else {
            *buffer += "~";
        }

        *buffer += "\x1b[K";
        if y < e.screenrows - 1 {
            *buffer += "\r\n";
        }
    }
}
/*** input ***/

fn editor_move_cursor(key: EditorKey, e: &mut EditorConfig) {
    match key {
        EditorKey::ArrowLeft if e.cx > 0 => e.cx -= 1,
        EditorKey::ArrowRight if e.cx < e.screencols - 1 => e.cx += 1,
        EditorKey::ArrowUp if e.cy > 0 => e.cy -= 1,
        EditorKey::ArrowDown if e.cy < e.screenrows - 1 => e.cy += 1,
        _ => (),
    }
}

fn editor_process_keypress(e: &mut EditorConfig) {
    let key = editor_read_key();
    match key {
        EditorKey::Char(c) if c == ctrl_key(b'q') => {
            print!("{}", "\x1b[2J\x1b[H");
            flush_stdout();
            exit(0);
        }

        EditorKey::ArrowDown
        | EditorKey::ArrowUp
        | EditorKey::ArrowLeft
        | EditorKey::ArrowRight => editor_move_cursor(key, e),

        EditorKey::PageDown | EditorKey::PageUp => {
            for _ in 0..e.screenrows {
                editor_move_cursor(
                    if key == EditorKey::PageUp {
                        EditorKey::ArrowUp
                    } else {
                        EditorKey::ArrowDown
                    },
                    e,
                );
            }
        }

        EditorKey::Home => e.cx = 0,
        EditorKey::End => e.cx = e.screencols - 1,

        _ => (),
    }
}

/*** init ***/

fn init_editor() -> EditorConfig {
    EditorConfig::from_env()
        .map_err(|e| die(&format!("init_editor: {}", e)))
        .unwrap()
}

fn main() {
    enable_raw_mode();
    let mut e = init_editor();

    loop {
        editor_refresh_screen(&e);
        editor_process_keypress(&mut e);
    }
}
