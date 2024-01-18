mod bootrom;
mod bl2;

use std::io::{BufRead, BufReader};
use clap::Parser;
use clap_num::maybe_hex;
use std::time::Duration;
use serialport::SerialPort;

/// Utility to upload and execute binaries over UART for Mediatek SoCs.
#[derive(Parser, Debug)]
struct Args {
    /// Serial port
    #[arg(short, long)]
    serial: String,

    /// Path to the binary code to be executed
    #[arg(short, long)]
    payload: String,

    /// Load address of the payload
    #[arg(short, long, value_parser=maybe_hex::<u32>, default_value_t = 0x201000)]
    load_addr: u32,

    /// Path to an additional ARMv7 payload. If specified, both payloads will be loaded and execution starts at this one.
    #[arg(short, long)]
    a32_payload: Option<String>,

    /// Load address of the payload
    #[arg(long, value_parser=maybe_hex::<u32>, default_value_t = 0x200a00)]
    a32_load_addr: u32,

    /// Path to an FIP payload. When using MTK BL2 built with UART download support
    #[arg(short, long)]
    fip: Option<String>,

    /// Load address of the payload
    #[arg(long, default_value_t = 921600)]
    bl2_load_baudrate: u32,
}

fn load_bl2(args: &Args, port: Box<dyn SerialPort>) -> Box<dyn SerialPort> {
    let mut brom_dev = bootrom::BootROM::new(port);

    println!("Handshake...");
    brom_dev.handshake();
    let hw_code = brom_dev.get_hw_code();
    println!("hw code: {:#x}", hw_code);
    let (hw_sub_code, hw_ver, sw_ver) = brom_dev.get_hw_dict();
    println!("hw sub code: {:#x}", hw_sub_code);
    println!("hw ver: {:#x}", hw_ver);
    println!("sw ver: {:#x}", sw_ver);

    let (sb, sla, daa) = brom_dev.get_target_config();
    if sb {
        panic!("Secure boot enabled.");
    }
    if sla {
        panic!("Serial link authorization enabled.");
    }
    if daa {
        panic!("Download agent authorization enabled.")
    }

    let payload = std::fs::read(&args.payload)
        .expect("failed to open payload.");
    println!("sending payload to {:#x}...", args.load_addr);
    let checksum = brom_dev.send_da(args.load_addr, 0, payload.as_slice());
    println!("Checksum: {:#x}", checksum);

    match &args.a32_payload {
        None => {
            println!("Jumping to {:#x}...", args.load_addr);
            brom_dev.jump_da(args.load_addr);
        }
        Some(a32_path) => {
            let a32_payload = std::fs::read(a32_path)
                .expect("failed to open payload.");
            println!("sending a32 payload to {:#x}...", args.a32_load_addr);
            let a32_checksum = brom_dev.send_da(args.a32_load_addr, 0, a32_payload.as_slice());
            println!("Checksum: {:#x}", a32_checksum);
            println!("Jumping to {:#x}...", args.a32_load_addr);
            brom_dev.jump_da(args.a32_load_addr);
        }
    }

    brom_dev.into_serial_port()
}

fn wait_for_line(port: Box<dyn SerialPort>, pattern: &str) -> (bool, Box<dyn SerialPort>) {
    let mut reader = BufReader::new(port);
    let mut uart_line = String::new();
    let mut ret = false;
    println!("==================================");
    while let Ok(_len) = reader.read_line(&mut uart_line) {
        print!("{}", uart_line);
        if uart_line.contains(pattern) {
            ret = true;
            break;
        }
        uart_line.clear();
    }
    println!("==================================");
    if !ret {
        println!("Timeout waiting for specified message.");
    }
    (ret, reader.into_inner())
}

fn wait_bl2_handshake(mut port: Box<dyn SerialPort>) -> (bool, Box<dyn SerialPort>) {
    port.set_timeout(Duration::from_secs(2)).unwrap();
    println!("Waiting for BL2. Message below:");
    wait_for_line(port, "Starting UART download handshake")
}

fn load_fip(port: Box<dyn SerialPort>, baudrate: u32, fip: &str) {
    let mut bl2_dev = bl2::BL2::new(port);
    bl2_dev.handshake();
    println!("BL2 UART DL version: {:#x}", bl2_dev.version());
    bl2_dev.set_baudrate(baudrate);
    bl2_dev.handshake();
    println!("Baud rate set to: {}", baudrate);

    let payload = std::fs::read(fip)
        .expect("failed to open fip.");
    bl2_dev.send_fip(&payload);
    println!("FIP sent.");

    bl2_dev.go();

    wait_for_line(bl2_dev.into_serial_port(), "Received FIP");
}

fn main() {
    let args = Args::parse();

    let port = serialport::new(&args.serial, 115200)
        .timeout(Duration::from_secs(2))
        .open().expect("Failed to open port");

    let port = load_bl2(&args, port);
    if let Some(fip_path) = &args.fip {
        let (handshake_result, port) = wait_bl2_handshake(port);
        if !handshake_result {
            return;
        }
        load_fip(port, args.bl2_load_baudrate, fip_path);
    }
}
