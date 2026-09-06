use std::{io::{self, Read, Write}, time::Duration};

fn main() {
    let executable = std::env::current_exe().unwrap();
    let mode = executable.file_stem().unwrap().to_str().unwrap();
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input).unwrap();
    match mode {
        "truncated" => io::stdout().write_all(b"CREMARES").unwrap(),
        "oversized" => {
            let mut output = b"CREMARES".to_vec();
            output.extend_from_slice(&1u16.to_le_bytes());
            output.push(0);
            output.extend_from_slice(&u32::MAX.to_le_bytes());
            output.extend_from_slice(&u64::MAX.to_le_bytes());
            io::stdout().write_all(&output).unwrap();
        },
        "exit" => std::process::exit(7),
        "timeout" => std::thread::sleep(Duration::from_secs(30)),
        "stderr" => loop { if io::stderr().write_all(&[b'x'; 8192]).is_err() { break; } },
        _ => panic!("unknown process test mode"),
    }
}
