use std::mem;
use std::io::Read;
use libc::{STDIN_FILENO, tcsetattr, tcgetattr, TCSAFLUSH, termios, ECHO};

fn enable_raw_mode() {
    let mut raw: termios = unsafe { mem::uninitialized() };
    unsafe {
        tcgetattr(STDIN_FILENO, &mut raw);
        raw.c_lflag &= !ECHO;
        tcsetattr(STDIN_FILENO, TCSAFLUSH, &mut raw);
    }
}

fn main() {
    enable_raw_mode();

    let mut c: [u8; 1] = [0];
    while std::io::stdin().read_exact(&mut c).is_ok() && c != ['q' as u8] {
    }
}