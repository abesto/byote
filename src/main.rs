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

/*** data ***/

struct EditorConfig {
    screenrows: u16,
    screencols: u16,
}

impl EditorConfig {
    fn from_env() -> Result<EditorConfig> {
        let (rows, cols) = get_window_size()?;
        Ok(EditorConfig {
            screenrows: rows,
            screencols: cols,
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

fn get_cursor_position() -> Result<(u16, u16)> {
    std::io::stdout().write_all(b"\x1b[6n\r\n")?;
    flush_stdout();

    let mut buffer = [0_u8; 32];
    let mut i = 0;
    while i < buffer.len() {
        if !std::io::stdin().read_exact(&mut buffer[i..=i]).is_ok() {
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
        rows_and_cols[0].parse::<u16>()?,
        rows_and_cols[1].parse::<u16>()?,
    ))
}

fn get_window_size() -> Result<(u16, u16)> {
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
        Ok((ws.ws_row, ws.ws_col))
    }
}

/*** output ***/
fn flush_stdout() {
    std::io::stdout()
        .flush()
        .map_err(|e| die(&format!("flush_stdout: {}", e)))
        .unwrap();
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
    for y in 1..e.screenrows {
        print!("~");
        if y < e.screenrows - 1 {
            print!("\r\n");
        }
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
        .map_err(|e| die(&format!("init_editor: {}", e)))
        .unwrap()
}

fn main() {
    enable_raw_mode();
    let e = init_editor();

    loop {
        editor_refresh_screen(&e);
        editor_process_keypress();
    }
}
