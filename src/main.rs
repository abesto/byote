use std::io::Read;
use libc::{STDIN_FILENO, TCSAFLUSH, ECHO, ICANON,
           tcsetattr, tcgetattr, termios,
           atexit};

// Couldn't find a way to convince the Rust compiler uninitialized global variables are OK.
// Guess that's a plus? Should be possible with std::MaybeUninit::uninitialized()
// at some point, maybe, possibly (see https://github.com/rust-lang/rust/issues/53491).
static mut ORIG_TERMIOS: termios = termios {
    c_cc: [0; 32],
    c_cflag: 0,
    c_iflag: 0,
    c_ispeed: 0,
    c_lflag: 0,
    c_line: 0,
    c_oflag: 0,
    c_ospeed: 0
};

extern "C" fn disable_raw_mode() {
    unsafe {
        tcsetattr(STDIN_FILENO, TCSAFLUSH, &mut ORIG_TERMIOS);
    }
}

unsafe fn enable_raw_mode() {
    tcgetattr(STDIN_FILENO, &mut ORIG_TERMIOS);
    atexit(disable_raw_mode);

    let mut raw = ORIG_TERMIOS;

    raw.c_lflag &= !(ECHO | ICANON);
    tcsetattr(STDIN_FILENO, TCSAFLUSH, &mut raw);
}

fn main() {
    unsafe {
        enable_raw_mode();
    }

    let mut c: [u8; 1] = [0];
    while std::io::stdin().read_exact(&mut c).is_ok() && c != ['q' as u8] {
    }
}