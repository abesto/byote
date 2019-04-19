/*** includes ***/

#[macro_use]
extern crate lazy_static;

use libc::{atexit, ioctl, winsize, TIOCGWINSZ};
use nix::Error;
use std::io::{BufRead, ErrorKind, Read, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::process::exit;
use termios::{
    tcsetattr, Termios, BRKINT, CS8, ECHO, ICANON, ICRNL, IEXTEN, INPCK, ISIG, ISTRIP, IXON, OPOST,
    TCSAFLUSH, VMIN, VTIME,
};

/*** defines ***/
fn ctrl_key(k: u8) -> u8 {
    k & 0x1f
}

/*** data ***/

struct EditorConfig {
    screenrows: u16,
    screencols: u16,
}

impl EditorConfig {
    fn from_env() -> EditorConfig {
        let mut ec = EditorConfig {
            screenrows: 0,
            screencols: 0,
        };
        if !get_window_size(&mut ec.screenrows, &mut ec.screencols) {
            die("EditorConfig::from_fd/get_window_size");
        }
        ec
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
    editor_clear_screen();
    print!("{}: {}\r\n", s, Error::last().to_string());
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

fn editor_read_key() -> u8 {
    let mut buffer: [u8; 1] = [0];
    loop {
        match std::io::stdin().read(&mut buffer) {
            Err(ref e) if e.kind() != ErrorKind::Interrupted => die("editor_read_key/read"),
            Ok(1) => return buffer[0],
            _ => (),
        }
    }
}

fn get_cursor_position(rows: &mut u16, cols: &mut u16) -> bool {
    let result = std::io::stdout().write(b"\x1b[6n");
    print!("\r\n");
    flush_stdout();
    if result.unwrap_or(0) != 4 {
        return false;
    }

    let mut buffer = [0_u8; 32];
    let mut i = 0;
    while i < buffer.len() {
        if std::io::stdin().read(&mut buffer[i..=i]).unwrap_or(0) != 1 {
            break;
        }
        if buffer[i] == b'R' {
            break;
        }
        i += 1;
    }

    let output = std::str::from_utf8(&buffer[0..i]).unwrap();
    if &output[0..=1] != "\x1b[" {
        die("get_cursor_position/invalid-response");
    }

    let rows_and_cols: Vec<&str> = output[2..i].split(';').collect();
    if (rows_and_cols.len() != 2) {
        die("get_cursor_position/split/len");
    }
    *rows = rows_and_cols[0].parse::<u16>().unwrap();
    *cols = rows_and_cols[1].parse::<u16>().unwrap();

    true
}

fn get_window_size(rows: &mut u16, cols: &mut u16) -> bool {
    let mut ws: winsize = unsafe { std::mem::zeroed() };
    let result = unsafe { ioctl(*STDOUT_RAWFD, TIOCGWINSZ, &mut ws) };
    if true || result == -1 || ws.ws_col == 0 {
        let result = std::io::stdout().write(b"\x1b[999C\x1b[999B");
        flush_stdout();
        match result {
            Ok(12) => {
                return get_cursor_position(rows, cols);
            }
            _ => return false,
        }
        false
    } else {
        *cols = ws.ws_col;
        *rows = ws.ws_row;
        true
    }
}

/*** output ***/
fn flush_stdout() {
    std::io::stdout()
        .flush()
        .unwrap_or_else(|_| die("editor_clear_screen/flush"));
}

fn editor_clear_screen() {
    print!("\x1b[2J");
    print!("\x1b[H");
}

fn editor_refresh_screen(e: &EditorConfig) {
    editor_clear_screen();
    editor_draw_rows(e);

    print!("\x1b[H");
    flush_stdout();
}

#[allow(clippy::print_with_newline)]
fn editor_draw_rows(e: &EditorConfig) {
    for _i in 1..e.screenrows {
        print!("~\r\n");
    }
}
/*** input ***/

fn editor_process_keypress() {
    let c = editor_read_key();
    if c == ctrl_key(b'q') {
        editor_clear_screen();
        exit(0);
    }
}

/*** init ***/

fn init_editor() -> EditorConfig {
    EditorConfig::from_env()
}

fn main() {
    enable_raw_mode();
    let e = init_editor();

    loop {
        editor_refresh_screen(&e);
        editor_process_keypress();
    }
}
