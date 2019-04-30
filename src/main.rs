/*** includes ***/

#[macro_use]
extern crate lazy_static;

#[macro_use]
extern crate simple_error;

use libc::{atexit, ioctl, winsize, TIOCGWINSZ};
use nix::Error;
use std::io::{BufRead, ErrorKind, Read, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::process::exit;
use std::vec::Vec;
use termios::{
    tcsetattr, Termios, BRKINT, CS8, ECHO, ICANON, ICRNL, IEXTEN, INPCK, ISIG, ISTRIP, IXON, OPOST,
    TCSAFLUSH, VMIN, VTIME,
};

/*** defines ***/
fn unwrap_or_die<T, E>(msg: &str, r: std::result::Result<T, E>) -> T
where
    E: std::fmt::Debug,
{
    r.map_err(|e| die(&format!("{}: {:#?}", msg, e))).unwrap()
}

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
    PageUp,
    Delete,
    End,
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
    rows: Vec<String>,
    rowoff: usize,
    coloff: usize,
}

impl EditorConfig {
    fn from_env() -> Result<EditorConfig> {
        let (rows, cols) = get_window_size()?;
        Ok(EditorConfig {
            screenrows: rows,
            screencols: cols,
            cx: 0,
            cy: 0,
            rows: Vec::new(),
            rowoff: 0,
            coloff: 0,
        })
    }
}

lazy_static! {
    static ref STDIN_RAWFD: RawFd = std::io::stdin().as_raw_fd();
    static ref STDOUT_RAWFD: RawFd = std::io::stdout().as_raw_fd();
    static ref ORIG_TERMIOS: Termios = unwrap_or_die(
        "lazy_static!/Termios::from_fd",
        Termios::from_fd(*STDIN_RAWFD)
    );
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

                    let seq = std::io::stdin()
                        .read(&mut buffer)
                        .map_err(Box::new)
                        .map(|n| &buffer[..n]);
                    if seq.is_err() {
                        return EditorKey::Escape;
                    }

                    let seq_str = unwrap_or_die(
                        "editor_read_key/from_utf8",
                        std::str::from_utf8(seq.unwrap()),
                    );

                    return match seq_str {
                        "[A" => EditorKey::ArrowUp,
                        "[B" => EditorKey::ArrowDown,
                        "[C" => EditorKey::ArrowRight,
                        "[D" => EditorKey::ArrowLeft,

                        "[3~" => EditorKey::Delete,
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

    let output = unwrap_or_die(
        "get_cursor_position/from_utf8",
        std::str::from_utf8(&buffer[0..i]),
    );
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
            x => bail!(format!("failed to determine window size: {:?}", x)),
        }
    } else {
        Ok((ws.ws_row as usize, ws.ws_col as usize))
    }
}

/*** row operations ***/

fn editor_append_row(e: &mut EditorConfig, s: String) {
    e.rows.push(s);
}

/*** file i/o ***/

fn editor_open(e: &mut EditorConfig, filename: &str) {
    let file = unwrap_or_die("editor_open/open", std::fs::File::open(filename));
    let reader = std::io::BufReader::new(file);
    for line in reader.lines() {
        line.map(|l| editor_append_row(e, l)).unwrap();
    }
}

/*** output ***/

fn editor_scroll(e: &mut EditorConfig) {
    if e.cy < e.rowoff {
        e.rowoff = e.cy;
    }
    if e.cy >= e.screenrows + e.rowoff {
        e.rowoff = e.cy - e.screenrows + 1;
    }
}

fn flush_stdout() {
    unwrap_or_die("flush_stdout", std::io::stdout().flush())
}

fn editor_refresh_screen(e: &mut EditorConfig) {
    editor_scroll(e);

    let mut buffer = String::new();

    buffer += "\x1b[?25l";
    buffer += "\x1b[H";

    editor_draw_rows(e, &mut buffer);

    buffer += &format!("\x1b[{};{}H", e.cy - e.rowoff + 1, e.cx + 1);

    buffer += "\x1b[?25h";

    print!("{}", buffer);

    flush_stdout();
}

#[allow(clippy::print_with_newline)]
fn editor_draw_rows(e: &EditorConfig, buffer: &mut String) {
    for y in 0..e.screenrows {
        let filerow = y + e.rowoff;
        if filerow >= e.rows.len() {
            if e.rows.is_empty() && y == e.screenrows / 3 {
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
        } else {
            let row = &e.rows[filerow];
            if !row.is_empty() {
                let len = e.screencols.min(row.len() - 1);
                *buffer += &row[..len];
            }
        }
        *buffer += "\x1b[K";
        if y < e.screenrows - 1 {
            *buffer += "\r\n";
        }
    }
}

/*** input ***/

fn editor_move_cursor(key: &EditorKey, e: &mut EditorConfig) {
    match key {
        EditorKey::ArrowLeft if e.cx > 0 => e.cx -= 1,
        EditorKey::ArrowRight if e.cx < e.screencols - 1 => e.cx += 1,
        EditorKey::ArrowUp if e.cy > 0 => e.cy -= 1,
        EditorKey::ArrowDown if e.cy < e.rows.len() - 1 => e.cy += 1,
        _ => (),
    }
}

fn editor_process_keypress(e: &mut EditorConfig) {
    let key = editor_read_key();
    match key {
        EditorKey::Char(c) if c == ctrl_key(b'q') => {
            print!("\x1b[2J\x1b[H");
            flush_stdout();
            exit(0);
        }

        EditorKey::ArrowDown
        | EditorKey::ArrowUp
        | EditorKey::ArrowLeft
        | EditorKey::ArrowRight => editor_move_cursor(&key, e),

        EditorKey::PageDown | EditorKey::PageUp => {
            let arrow = if key == EditorKey::PageUp {
                EditorKey::ArrowUp
            } else {
                EditorKey::ArrowDown
            };
            for _ in 0..e.screenrows {
                editor_move_cursor(&arrow, e);
            }
        }

        EditorKey::Home => e.cx = 0,
        EditorKey::End => e.cx = e.screencols - 1,

        _ => (),
    }
}

/*** init ***/

fn init_editor() -> EditorConfig {
    unwrap_or_die("init_editor", EditorConfig::from_env())
}

fn main() {
    enable_raw_mode();
    let mut e = init_editor();

    if let Some(filename) = std::env::args().nth(1) {
        editor_open(&mut e, &filename)
    }

    loop {
        editor_refresh_screen(&mut e);
        editor_process_keypress(&mut e);
    }
}
