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
use std::time::{Duration, Instant};
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
const BYOTE_TAB_STOP: usize = 8;

/*** data ***/

struct ERow {
    chars: String,
    render: String,
}

struct EditorConfig {
    screenrows: usize,
    screencols: usize,
    cx: usize,
    rx: usize,
    cy: usize,
    rows: Vec<ERow>,
    rowoff: usize,
    coloff: usize,
    filename: Option<String>,
    statusmsg: String,
    statusmsg_time: Instant,
}

impl EditorConfig {
    fn from_env() -> Result<EditorConfig> {
        let (rows, cols) = get_window_size()?;
        Ok(EditorConfig {
            screenrows: rows - 1,
            screencols: cols,
            cx: 0,
            rx: 0,
            cy: 0,
            rows: Vec::new(),
            rowoff: 0,
            coloff: 0,
            filename: None,
            statusmsg: String::new(),
            statusmsg_time: Instant::now(),
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

fn editor_row_cx_to_rx(r: &ERow, cx: usize) -> usize {
    let mut rx: usize = 0;
    for c in r.chars.chars().take(cx) {
        rx += match c {
            '\t' => BYOTE_TAB_STOP - (rx % BYOTE_TAB_STOP),
            _ => 1,
        }
    }
    rx
}

fn editor_update_row(r: &mut ERow) {
    r.render = r
        .chars
        .char_indices()
        .map(|(i, c)| match c {
            '\t' => " ".repeat(BYOTE_TAB_STOP - i % BYOTE_TAB_STOP),
            _ => c.to_string(),
        })
        .collect();
}

fn editor_append_row(e: &mut EditorConfig, s: String) {
    let mut row = ERow {
        chars: s,
        render: String::new(),
    };
    editor_update_row(&mut row);
    e.rows.push(row);
}

/*** file i/o ***/

fn editor_open(e: &mut EditorConfig, filename: &str) {
    e.filename = Some(filename.into());
    let file = unwrap_or_die("editor_open/open", std::fs::File::open(filename));
    let reader = std::io::BufReader::new(file);
    for line in reader.lines() {
        line.map(|l| editor_append_row(e, l)).unwrap();
    }
}

/*** output ***/

fn editor_scroll(e: &mut EditorConfig) {
    e.rx = e
        .rows
        .get(e.cy)
        .map(|r| editor_row_cx_to_rx(r, e.cx))
        .unwrap_or(0);

    if e.cy < e.rowoff {
        e.rowoff = e.cy;
    }
    if e.cy >= e.screenrows + e.rowoff {
        e.rowoff = e.cy - e.screenrows + 1;
    }
    if e.rx < e.coloff {
        e.coloff = e.rx;
    }
    if e.rx >= e.coloff + e.screencols {
        e.coloff = e.rx - e.screencols + 1;
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
    editor_draw_status_bar(e, &mut buffer);

    buffer += &format!("\x1b[{};{}H", e.cy - e.rowoff + 1, (e.rx - e.coloff) + 1);

    buffer += "\x1b[?25h";

    print!("{}", buffer);

    flush_stdout();
}

fn editor_set_status_message(e: &mut EditorConfig, msg: &str) {
    // Not taking a format string, because unlike C, variadics are hard, and also format! is easy
    // to use at the call-site
    e.statusmsg = msg.into();
    e.statusmsg_time = Instant::now();
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
            let len = row
                .render
                .len()
                .checked_sub(e.coloff)
                .unwrap_or(0)
                .min(e.screencols);
            if len > 0 {
                *buffer += &row.render[e.coloff..e.coloff + len];
            }
        }
        *buffer += "\x1b[K";
        *buffer += "\r\n";
    }
}

fn editor_draw_status_bar(e: &EditorConfig, buffer: &mut String) {
    *buffer += "\x1b[7m";

    let shown_filename: String = e
        .filename
        .clone()
        .map(|s| s.chars().take(20).collect())
        .unwrap_or_else(|| "[No Name]".into());
    let status = format!("{} - {} lines", shown_filename, e.rows.len());
    let rstatus = format!("{}/{}", e.cy + 1, e.rows.len());

    *buffer += &status[..=e.screencols.min(status.len() - 1)];
    *buffer += &" ".repeat(e.screencols - status.len() - rstatus.len());
    *buffer += &rstatus;
    *buffer += "\x1b[m";
}

/*** input ***/

fn editor_move_cursor(key: &EditorKey, e: &mut EditorConfig) {
    let row_old = e.rows.get(e.cy).map(|r| &r.chars);
    let rowlen_old = row_old.map(String::len).unwrap_or(0);
    match key {
        EditorKey::ArrowLeft if e.cx > 0 => e.cx -= 1,
        EditorKey::ArrowLeft if e.cy > 0 => {
            e.cy -= 1;
            e.cx = e.rows[e.cy].chars.len();
        }
        EditorKey::ArrowRight if e.cx < rowlen_old => e.cx += 1,
        EditorKey::ArrowRight if row_old.is_some() && rowlen_old == e.cx => {
            e.cy += 1;
            e.cx = 0;
        }
        EditorKey::ArrowUp if e.cy > 0 => e.cy -= 1,
        EditorKey::ArrowDown if e.cy < e.rows.len() - 1 => e.cy += 1,
        _ => (),
    }

    let row_new = e.rows.get(e.cy).map(|r| &r.chars);
    let rowlen_new = row_new.map(String::len).unwrap_or(0);
    if e.cx > rowlen_new {
        e.cx = rowlen_new;
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
                e.cy = e.rowoff;
                EditorKey::ArrowUp
            } else {
                e.cy = e.rows.len().min(e.rowoff + e.screenrows - 1);
                EditorKey::ArrowDown
            };
            for _ in 0..e.screenrows {
                editor_move_cursor(&arrow, e);
            }
        }

        EditorKey::Home => e.cx = 0,
        EditorKey::End => {
            if e.cy < e.rows.len() {
                e.cx = e.rows[e.cy].chars.len();
            }
        }

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

    editor_set_status_message(&mut e, "HELP: Ctrl-Q = quit");

    if let Some(filename) = std::env::args().nth(1) {
        editor_open(&mut e, &filename)
    }

    loop {
        editor_refresh_screen(&mut e);
        editor_process_keypress(&mut e);
    }
}
