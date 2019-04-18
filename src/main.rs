use std::io::Read;

fn main() {
    let mut c: [u8; 1] = [0];
    while std::io::stdin().read_exact(&mut c).is_ok() && c != ['q' as u8] {
    }
}