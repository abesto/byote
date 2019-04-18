#[macro_use]
extern crate lazy_static;

use std::io::Read;
use std::os::unix::io::{AsRawFd, RawFd};
use termios::{Termios, tcsetattr, TCSAFLUSH, ECHO, ICANON};
use libc::atexit;

lazy_static! {
    static ref STDIN: RawFd = std::io::stdin().as_raw_fd();
    static ref ORIG_TERMIOS: Termios = Termios::from_fd(*STDIN).unwrap();
}

extern "C" fn disable_raw_mode() {
    tcsetattr(*STDIN, TCSAFLUSH, &*ORIG_TERMIOS).unwrap();
}

fn enable_raw_mode() {
    unsafe {
        atexit(disable_raw_mode);
    };

    let mut raw: Termios = *ORIG_TERMIOS;
    raw.c_lflag &= !(ECHO | ICANON);
    tcsetattr(*STDIN, TCSAFLUSH, &mut raw).unwrap();
}

fn main() {
    enable_raw_mode();

    let mut c: [u8; 1] = [0];
    while std::io::stdin().read_exact(&mut c).is_ok() && c != ['q' as u8] {
    }
}