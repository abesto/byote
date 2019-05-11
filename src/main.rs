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

type PromptCallback = fn(&mut EditorConfig, &str, &EditorKey);

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

    Return,
    Escape,

    Char(u8),
}

const BYOTE_VERSION: Option<&'static str> = option_env!("CARGO_PKG_VERSION");
const BYOTE_TAB_STOP: usize = 8;
const BYOTE_QUIT_TIMES: u8 = 3;

const BACKSPACE: u8 = 127;

fn is_backspace_or_delete(k: &EditorKey) -> bool {
    match *k {
        EditorKey::Delete => true,
        EditorKey::Char(c) if c == BACKSPACE || c == ctrl_key(b'h') => true,
        _ => false,
    }
}

/*** data ***/

#[derive(Ord, PartialOrd, Eq, PartialEq, Clone)]
enum Highlight {
    Normal,
    Number,
    Match,
}

struct ERow {
    chars: String,
    render: String,
    hl: Vec<Highlight>,
}

struct FindState {
    last_match: isize,
    direction: i8,
    saved_hl_line: usize,
    saved_hl: Option<Vec<Highlight>>,
}

struct EditorConfig {
    screenrows: usize,
    screencols: usize,
    cx: usize,
    rx: usize,
    cy: usize,
    rows: Vec<ERow>,
    dirty: bool,
    quit_times: u8,
    rowoff: usize,
    coloff: usize,
    filename: Option<String>,
    statusmsg: String,
    statusmsg_time: Instant,
    find: FindState,
}

impl EditorConfig {
    fn from_env() -> Result<EditorConfig> {
        let (rows, cols) = get_window_size()?;
        Ok(EditorConfig {
            screenrows: rows - 2,
            screencols: cols,
            cx: 0,
            rx: 0,
            cy: 0,
            rows: Vec::new(),
            dirty: false,
            quit_times: 3,
            rowoff: 0,
            coloff: 0,
            filename: None,
            statusmsg: String::new(),
            statusmsg_time: Instant::now(),
            find: FindState {
                last_match: -1,
                direction: 1,
                saved_hl_line: 0,
                saved_hl: None,
            },
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
                } else if c == b'\r' {
                    return EditorKey::Return;
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

/*** syntax highlighting ***/

fn editor_update_syntax(row: &mut ERow) {
    row.hl = vec![Highlight::Normal; row.render.len()];

    for (i, c) in row.render.char_indices() {
        if c.is_ascii_digit() {
            row.hl[i] = Highlight::Number;
        }
    }
}

fn editor_syntax_to_color(hl: &Highlight) -> u8 {
    match hl {
        Highlight::Number => 31,
        Highlight::Match => 34,
        _ => 37,
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

fn editor_row_rx_to_cx(r: &ERow, rx: usize) -> usize {
    let mut cur_rx = 0;
    for (cx, c) in r.render.char_indices() {
        cur_rx += match c {
            '\t' => BYOTE_TAB_STOP - (cur_rx % BYOTE_TAB_STOP),
            _ => 1,
        };
        if cur_rx > rx {
            return cx;
        };
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

    editor_update_syntax(r);
}

fn editor_insert_row(e: &mut EditorConfig, at: usize, s: &str) {
    if at > e.rows.len() {
        return;
    }

    let mut row = ERow {
        chars: String::from(s),
        render: String::new(),
        hl: Vec::new(),
    };
    editor_update_row(&mut row);

    e.rows.insert(at, row);
    e.dirty = true;
}

fn editor_del_row(e: &mut EditorConfig, at: usize) {
    if at >= e.rows.len() {
        // note usize can never be < 0, so not checking that
        return;
    }
    e.rows.remove(at);
    e.dirty = true;
}

fn editor_row_insert_char(e: &mut EditorConfig, at: usize, c: char) {
    let row = &mut e.rows[e.cy];
    row.chars.insert(at.max(0).min(row.chars.len()), c);
    editor_update_row(row);
    e.dirty = true;
}

fn editor_row_append_string(e: &mut EditorConfig, at_row: usize, s: &str) {
    let row = &mut e.rows[at_row];
    row.chars += s;
    editor_update_row(row);
    e.dirty = true;
}

fn editor_row_del_char(e: &mut EditorConfig, at_row: usize, at: usize) {
    let row = &mut e.rows[at_row];
    row.chars.remove(at.max(0).min(row.chars.len()));
    editor_update_row(row);
    e.dirty = true;
}

/*** editor operations ***/

fn editor_insert_char(e: &mut EditorConfig, c: char) {
    if e.cy == e.rows.len() {
        editor_insert_row(e, e.rows.len(), "");
    }
    editor_row_insert_char(e, e.cx, c);
    e.cx += 1;
}

fn editor_insert_new_line(e: &mut EditorConfig) {
    if e.cx == 0 {
        editor_insert_row(e, e.cy, "");
    } else {
        let right: String = e.rows[e.cy].chars[e.cx..].into();
        editor_insert_row(e, e.cy + 1, &right);
        let row = &mut e.rows[e.cy];
        row.chars = row.chars[..e.cx].into();
        editor_update_row(row);
    }
    e.cy += 1;
    e.cx = 0;
}

fn editor_del_char(e: &mut EditorConfig) {
    if e.cy == e.rows.len() {
        return;
    }
    if e.cx == 0 && e.cy == 0 {
        return;
    }
    if e.cx > 0 {
        editor_row_del_char(e, e.cy, e.cx - 1);
        e.cx -= 1;
    } else {
        e.cx = e.rows[e.cy - 1].chars.len();
        // This is clunky due to the fact that all of `e` needs to be borrowed,
        // and we can only borrow it mutably once, and we can't mix mutable
        // and immutable borrows of it. Note that `&e.blah` tries to borrow `e` fully.
        editor_row_append_string(e, e.cy - 1, &e.rows[e.cy].chars.clone());
        editor_del_row(e, e.cy);
        e.cy -= 1;
    }
}

/*** file i/o ***/

fn editor_rows_to_string(e: &EditorConfig) -> String {
    if e.rows.is_empty() {
        return String::new();
    }
    e.rows
        .iter()
        .skip(1)
        .map(|r| &r.chars)
        .fold(e.rows[0].chars.clone(), |a, b| a + "\n" + b)
}

fn editor_open(e: &mut EditorConfig, filename: &str) {
    e.filename = Some(filename.into());
    let file = unwrap_or_die("editor_open/open", std::fs::File::open(filename));
    let reader = std::io::BufReader::new(file);
    for line in reader.lines() {
        line.map(|l| editor_insert_row(e, e.rows.len(), &l))
            .unwrap();
    }
    e.dirty = false;
}

fn editor_save(e: &mut EditorConfig) {
    if e.filename.is_none() {
        e.filename = editor_prompt(e, "Save as (ESC to cancel): ", None);
        if e.filename.is_none() {
            editor_set_status_message(e, "Save aborted!");
            return;
        }
    }

    match &e.filename {
        Some(filename) => {
            let buf = editor_rows_to_string(e);
            let msg = &std::fs::File::create(filename)
                .and_then(|file| file.set_len(buf.len() as u64).map(|_| file))
                .and_then(|mut file| file.write(buf.as_bytes()))
                .map(|n| {
                    e.dirty = false;
                    format!("{} bytes written to disk", n)
                })
                .unwrap_or_else(|e| format!("Can't save! I/O error: {}", e));
            editor_set_status_message(e, msg);
        }
        None => (),
    }
}

/*** find ***/

fn editor_find_callback(e: &mut EditorConfig, query: &str, key: &EditorKey) {
    if let Some(saved_hl) = &e.find.saved_hl {
        e.rows[e.find.saved_hl_line].hl = saved_hl.clone();
        e.find.saved_hl = None;
    }

    match key {
        EditorKey::Escape | EditorKey::Return => {
            e.find.last_match = -1;
            e.find.direction = 1;
            return;
        }
        EditorKey::ArrowLeft | EditorKey::ArrowUp => e.find.direction = -1,
        EditorKey::ArrowDown | EditorKey::ArrowRight => e.find.direction = 1,
        _ => {
            e.find.last_match = -1;
            e.find.direction = 1;
        }
    }

    if e.find.last_match == -1 {
        e.find.direction = 1;
    }

    let mut current = e.find.last_match;
    for _y in 0..e.rows.len() {
        current += e.find.direction as isize;
        if current == -1 {
            current = (e.rows.len() - 1) as isize;
        } else if current == e.rows.len() as isize {
            current = 0;
        }
        let row = &e.rows[current as usize];
        match row.render.find(&query) {
            None => (),
            Some(rx) => {
                e.find.last_match = current;
                e.cy = current as usize;
                e.cx = editor_row_rx_to_cx(row, rx);
                e.rowoff = e.rows.len();

                e.find.saved_hl_line = current as usize;
                e.find.saved_hl = Some(e.rows[e.cy].hl.clone());
                e.rows[e.cy]
                    .hl
                    .splice(rx..rx + query.len(), vec![Highlight::Match; query.len()]);
                break;
            }
        }
    }
}

fn editor_find(e: &mut EditorConfig) {
    let saved_cx = e.cx;
    let saved_cy = e.cy;
    let saved_rowoff = e.rowoff;
    let saved_coloff = e.coloff;

    if editor_prompt(
        e,
        "Search (Use ESC/Arrows/Enter): ",
        Some(editor_find_callback),
    )
    .is_none()
    {
        e.cx = saved_cx;
        e.cy = saved_cy;
        e.rowoff = saved_rowoff;
        e.coloff = saved_coloff;
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
    editor_draw_message_bar(e, &mut buffer);

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
                let s = &row.render[e.coloff..len];
                let hls = &row.hl[e.coloff..len];
                let mut current_color: i8 = -1;
                for (c, hl) in s.chars().zip(hls) {
                    if *hl == Highlight::Normal {
                        if current_color != -1 {
                            *buffer += "\x1b[39m";
                            current_color = -1;
                        }
                        buffer.push(c);
                    } else {
                        let color = editor_syntax_to_color(hl);
                        if current_color as u8 != color {
                            current_color = color as i8;
                            *buffer += &format!("\x1b[{}m", color);
                        }
                        buffer.push(c);
                    }
                }
            }

            *buffer += "\x1b[39m";
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
    let status = format!(
        "{} - {} lines {}",
        shown_filename,
        e.rows.len(),
        if e.dirty { "(modified)" } else { "" }
    );
    let rstatus = format!("{}/{}", e.cy + 1, e.rows.len());

    *buffer += &status[..=e.screencols.min(status.len() - 1)];
    *buffer += &" ".repeat(e.screencols - status.len() - rstatus.len());
    *buffer += &rstatus;
    *buffer += "\x1b[m";
    *buffer += "\r\n";
}

fn editor_draw_message_bar(e: &EditorConfig, buffer: &mut String) {
    *buffer += "\x1b[K";
    if !e.statusmsg.is_empty() && Instant::now() < e.statusmsg_time + Duration::from_secs(5) {
        let cropped_msg: String = e.statusmsg.chars().take(e.screencols).collect();
        *buffer += &cropped_msg;
    }
}

/*** input ***/

fn editor_prompt(
    e: &mut EditorConfig,
    prompt: &str,
    callback: Option<PromptCallback>,
) -> Option<String> {
    let mut buf = String::with_capacity(128);
    loop {
        editor_set_status_message(e, &format!("{}{}", prompt, &buf));
        editor_refresh_screen(e);
        let k = editor_read_key();
        match k {
            ref k if is_backspace_or_delete(k) && !buf.is_empty() => {
                buf.remove(buf.len() - 1);
            }
            EditorKey::Escape => {
                editor_set_status_message(e, "");
                if let Some(f) = callback {
                    f(e, &buf, &k)
                };
                return None;
            }
            EditorKey::Return => {
                if !buf.is_empty() {
                    editor_set_status_message(e, "");
                    if let Some(f) = callback {
                        f(e, &buf, &k)
                    };
                    return Some(buf);
                }
            }
            EditorKey::Char(c) if !c.is_ascii_control() && c < 128 => {
                // Strictly speaking we don't need to do this, but it's fun!
                if buf.len() == buf.capacity() - 1 {
                    buf.reserve(buf.len());
                }
                buf.push(c as char);
            }
            _ => (),
        }
        if let Some(f) = callback {
            f(e, &buf, &k)
        };
    }
}

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
        EditorKey::Return => editor_insert_new_line(e),

        EditorKey::Char(c) if c == ctrl_key(b'q') => {
            if e.dirty && e.quit_times > 0 {
                editor_set_status_message(
                    e,
                    &format!(
                        "WARNING!!! File has unsaved changes. Press Ctrl-Q {} more times to quit.",
                        e.quit_times
                    ),
                );
                e.quit_times -= 1;
                return;
            }
            print!("\x1b[2J\x1b[H");
            flush_stdout();
            exit(0);
        }

        EditorKey::Char(c) if c == ctrl_key(b's') => editor_save(e),

        EditorKey::ArrowDown
        | EditorKey::ArrowUp
        | EditorKey::ArrowLeft
        | EditorKey::ArrowRight => editor_move_cursor(&key, e),

        EditorKey::Home => e.cx = 0,
        EditorKey::End => {
            if e.cy < e.rows.len() {
                e.cx = e.rows[e.cy].chars.len();
            }
        }

        EditorKey::Char(c) if c == ctrl_key(b'f') => editor_find(e),

        ref k if is_backspace_or_delete(k) => {
            if *k == EditorKey::Delete {
                editor_move_cursor(&EditorKey::ArrowRight, e);
            }
            editor_del_char(e);
        }

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

        EditorKey::Char(c) if c == ctrl_key(b'l') || c == b'\x1b' => (),

        EditorKey::Char(c) => editor_insert_char(e, c.into()),
        _ => (),
    }

    e.quit_times = BYOTE_QUIT_TIMES;
}

/*** init ***/

fn init_editor() -> EditorConfig {
    unwrap_or_die("init_editor", EditorConfig::from_env())
}

fn main() {
    enable_raw_mode();
    let mut e = init_editor();

    editor_set_status_message(
        &mut e,
        "HELP: Ctrl-S = save | Ctrl-Q = quit | Ctrl-F = find",
    );

    if let Some(filename) = std::env::args().nth(1) {
        editor_open(&mut e, &filename)
    }

    loop {
        editor_refresh_screen(&mut e);
        editor_process_keypress(&mut e);
    }
}
