/*** includes ***/

#[macro_use]
extern crate lazy_static;

use libc::{atexit, ioctl, perror, winsize, TIOCGWINSZ};
use std::ffi::CString;
use std::io::{ErrorKind, Read, Write};
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
    orig_termios: Termios,
    screenrows: u16,
    screencols: u16,
}

impl EditorConfig {
    fn from_env() -> EditorConfig {
        let mut ec = EditorConfig {
            orig_termios: Termios::from_fd(*STDIN_RAWFD)
                .map_err(|_| die("EditorConfig::from_fb/orig_termios"))
                .unwrap(),
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
    static ref E: EditorConfig = EditorConfig::from_env();
}

/*** terminal ***/

fn die(s: &str) {
    editor_clear_screen();

    let cs = CString::new(s).expect("CString::new failed");
    unsafe {
        perror(cs.as_ptr() as *const i8);
    }
    exit(1);
}

extern "C" fn disable_raw_mode() {
    if tcsetattr(*STDIN_RAWFD, TCSAFLUSH, &E.orig_termios).is_err() {
        die("disable_raw_mode/tcsetattr");
    }
}

fn enable_raw_mode() {
    unsafe {
        atexit(disable_raw_mode);
    };

    let mut raw: Termios = E.orig_termios;
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
            Ok(_) => return buffer[0],
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
    let mut buffer = [0];
    while (std::io::stdin().read(&mut buffer).unwrap_or(0) == 1) {
        let c = buffer[0];
        if (c.is_ascii_control()) {
            print!("{}\r\n", c);
        } else {
            print!("{} ('{}')\r\n", c, c as char);
        }
        flush_stdout();
    }
    editor_read_key();
    return false;
}

fn get_window_size(rows: &mut u16, cols: &mut u16) -> bool {
    let mut ws: winsize = unsafe { std::mem::zeroed() };
    let result = unsafe { ioctl(*STDOUT_RAWFD, TIOCGWINSZ, &mut ws) };
    if true || result == -1 || ws.ws_col == 0 {
        let result = std::io::stdout().write(b"\x1b[999C\x1b[999B");
        flush_stdout();
        match result {
            Ok(12) => {
                get_cursor_position(rows, cols);
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

fn editor_refresh_screen() {
    editor_clear_screen();
    editor_draw_rows();

    print!("\x1b[H");
    flush_stdout();
}

#[allow(clippy::print_with_newline)]
fn editor_draw_rows() {
    for _i in 1..E.screenrows {
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

fn main() {
    enable_raw_mode();

    loop {
        editor_refresh_screen();
        editor_process_keypress();
    }
}
