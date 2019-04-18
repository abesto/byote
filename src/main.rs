use std::io::Read;
use termios::{Termios, ECHO, tcsetattr, TCSAFLUSH};
use std::os::unix::io::AsRawFd;

fn enable_raw_mode() {
    let fd = std::io::stdin().as_raw_fd();
    let mut termios = Termios::from_fd(fd).unwrap();
    termios.c_lflag &= !ECHO;
    tcsetattr(fd, TCSAFLUSH, &mut termios).unwrap();
}

fn main() {
    enable_raw_mode();

    let mut c: [u8; 1] = [0];
    while std::io::stdin().read_exact(&mut c).is_ok() && c != ['q' as u8] {
    }
}