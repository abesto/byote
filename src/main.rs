#[macro_use]
extern crate lazy_static;

use std::io::Read;
use std::os::unix::io::{AsRawFd, RawFd};
use termios::{Termios, tcsetattr, TCSAFLUSH, ECHO, ICANON, ISIG, IXON, IEXTEN, ICRNL, OPOST,
              BRKINT, INPCK, ISTRIP, CS8, VMIN, VTIME};
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
    raw.c_iflag &= !(BRKINT | ICRNL | INPCK | ISTRIP | IXON);
    raw.c_oflag &= !(OPOST);
    raw.c_cflag |= CS8;
    raw.c_lflag &= !(ECHO | ICANON | IEXTEN | ISIG);
    raw.c_cc[VMIN] = 0;
    raw.c_cc[VTIME] = 1;
    tcsetattr(*STDIN, TCSAFLUSH, &mut raw).unwrap();
}

fn main() {
    enable_raw_mode();

    let exit = ['q' as u8];
    let mut stdin = std::io::stdin();

    loop {
        let mut buffer: [u8; 1] = [0];
        stdin.read(&mut buffer).unwrap();
        let n = buffer[0];
        let c = n as char;
        if c.is_ascii_control() {
            print!("{}\r\n", n)
        } else {
            print!("{} ('{}')\r\n", n, c);
        }
        if buffer == exit {
            break;
        }
    }
}