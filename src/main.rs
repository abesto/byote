/*** includes ***/

#[macro_use]
extern crate lazy_static;

use std::io::{Read, ErrorKind};
use std::process::exit;
use std::ffi::CString;
use std::os::unix::io::{AsRawFd, RawFd};
use termios::{Termios, tcsetattr, TCSAFLUSH, ECHO, ICANON, ISIG, IXON, IEXTEN, ICRNL, OPOST,
              BRKINT, INPCK, ISTRIP, CS8, VMIN, VTIME};
use libc::{atexit, perror};

/*** defines ***/
fn ctrl_key(k: u8) -> u8 {
    return k & 0x1f;
}

/*** data ***/

lazy_static! {
    static ref STDIN_RAWFD: RawFd = std::io::stdin().as_raw_fd();
    static ref ORIG_TERMIOS: Termios = Termios::from_fd(*STDIN_RAWFD).map_err(|_| die("ORIG_TERMIOS")).unwrap();
}

/*** terminal ***/

fn die(s: &str) {
    let cs = CString::new(s).expect("CString::new failed");
    unsafe {
        perror(cs.as_ptr());
    }
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
    if tcsetattr(*STDIN_RAWFD, TCSAFLUSH, &mut raw).is_err() {
        die("enable_raw_mode/tcsetattr");
    }
}

fn editor_read_key() -> u8 {
    let mut buffer: [u8; 1] = [0];
    loop {
        match std::io::stdin().read(&mut buffer) {
            Err(ref e) if e.kind() != ErrorKind::Interrupted => die("editor_read_key/read"),
            Ok(_) => return buffer[0],
            _ => ()
        }
    }
}

/*** input ***/

fn editor_process_keypress() {
    let c = editor_read_key();
    if c == ctrl_key(b'q') {
        exit(0);
    }
}

/*** init ***/

fn main() {
    enable_raw_mode();

    loop {
        editor_process_keypress();
    }
}