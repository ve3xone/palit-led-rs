//! palit-led: standalone Palit GPU LED control via NvAPI I2C.
//! No ThPanel, no G-PANEL, no registry.

use palit_led::config::{self, Config};
use palit_led::led;
use palit_led::nvapi::NvApi;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const HELP: &str = "\
palit-led - Palit GPU LED control (NvAPI I2C, no ThPanel)

USAGE:
  palit-led <COMMAND> [OPTIONS]

COMMANDS:
  <RRGGBB>          set solid static color (hex), e.g. palit-led FF0000
  off               turn LEDs off
  temp              follow GPU temperature (green/yellow/red) - runs until Ctrl+C
  rainbow           smooth color carousel - runs until Ctrl+C
  id                read LED controller firmware id
  dump              dump key controller registers
  config            write default palit-led.toml next to the exe

OPTIONS:
  --br <0-100>      brightness (static)
  --gpu <sel>       'all' or comma list like 0,1  (default: config / first GPU)

CONFIG:
  palit-led.toml (next to exe) sets default GPUs and temp/rainbow params.
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        print!("{HELP}");
        return;
    }

    if let Err(e) = run(&args) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn parse_color(s: &str) -> Result<(u8, u8, u8), String> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return Err(format!("expected RRGGBB hex, got {s:?}"));
    }
    let v = u32::from_str_radix(s, 16).map_err(|_| format!("bad hex {s:?}"))?;
    Ok((((v >> 16) & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8))
}

fn opt_value(args: &[String], key: &str) -> Option<String> {
    args.iter().position(|a| a == key).and_then(|i| args.get(i + 1).cloned())
}

fn select_gpus(args: &[String], cfg: &Config, count: usize) -> Vec<usize> {
    if let Some(v) = opt_value(args, "--gpu") {
        if v.eq_ignore_ascii_case("all") {
            return (0..count).collect();
        }
        let list: Vec<usize> = v
            .split(',')
            .filter_map(|x| x.trim().parse().ok())
            .filter(|&i| i < count)
            .collect();
        if !list.is_empty() {
            return list;
        }
    }
    let sel = cfg.gpus.resolve(count);
    if sel.is_empty() {
        vec![0.min(count.saturating_sub(1))]
    } else {
        sel
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let cmd = args[0].as_str();

    if cmd == "config" {
        let p = config::write_default().map_err(|e| e.to_string())?;
        println!("wrote {}", p.display());
        return Ok(());
    }

    let cfg = config::load();
    let nv = NvApi::new()?;
    let count = nv.gpu_count();
    if count == 0 {
        return Err("no NVIDIA GPU found".into());
    }
    let gpus = select_gpus(args, &cfg, count);
    println!("GPUs: {count} present, targeting {gpus:?}");

    match cmd {
        "off" => {
            for &g in &gpus {
                led::off(&nv, g)?;
                println!("GPU{g}: off");
            }
        }
        "id" => {
            for &g in &gpus {
                println!("GPU{g} fw id: {:?}", led::read_fw_id(&nv, g)?);
            }
        }
        "dump" => {
            for &g in &gpus {
                println!("GPU{g}:");
                for (reg, d) in led::dump(&nv, g)? {
                    let hex: Vec<String> = d.iter().map(|b| format!("{b:02X}")).collect();
                    println!("  0x{reg:02X}: {}", hex.join(" "));
                }
            }
        }
        "temp" => run_temp(&nv, &gpus, &cfg)?,
        "rainbow" => run_rainbow(&nv, &gpus, &cfg)?,
        _ => {
            let (r, g, b) = parse_color(cmd)?;
            let br = opt_value(args, "--br")
                .and_then(|v| v.parse::<u8>().ok())
                .unwrap_or(100)
                .min(100);
            for &gi in &gpus {
                led::set_color(&nv, gi, r, g, b, br)?;
                println!("GPU{gi}: #{r:02X}{g:02X}{b:02X} @ {br}%");
            }
        }
    }
    Ok(())
}

fn stop_flag() -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    let _ = ctrlc::set_handler(move || s.store(true, Ordering::SeqCst));
    stop
}

fn run_temp(nv: &NvApi, gpus: &[usize], cfg: &Config) -> Result<(), String> {
    let t = &cfg.temp;
    println!(
        "temp mode: green<={}C, yellow<={}C, orange<={}C, red above; {}; interval {}ms. Ctrl+C to stop.",
        t.green_max,
        t.yellow_max,
        t.orange_max,
        if t.smooth { "smooth" } else { "zones" },
        t.interval_ms
    );
    let stop = stop_flag();
    let mut last: Vec<(u8, u8, u8)> = vec![(1, 1, 1); gpus.len()];
    while !stop.load(Ordering::SeqCst) {
        for (i, &g) in gpus.iter().enumerate() {
            let temp = nv.gpu_temp(g).unwrap_or(0);
            let c = led::temp_to_color(
                temp, t.green_max, t.yellow_max, t.orange_max, t.red_full, t.smooth,
            );
            if c != last[i] {
                led::set_color(nv, g, c.0, c.1, c.2, 100)?;
                last[i] = c;
                println!("GPU{g}: {temp}C -> #{:02X}{:02X}{:02X}", c.0, c.1, c.2);
            }
        }
        sleep_interruptible(t.interval_ms, &stop);
    }
    println!("\nstopped.");
    Ok(())
}

fn run_rainbow(nv: &NvApi, gpus: &[usize], cfg: &Config) -> Result<(), String> {
    let r = &cfg.rainbow;
    println!(
        "rainbow mode: step {}deg, interval {}ms, brightness {}%. Ctrl+C to stop.",
        r.step_deg, r.interval_ms, r.brightness
    );
    let stop = stop_flag();
    let mut hue = 0.0f32;
    while !stop.load(Ordering::SeqCst) {
        let (cr, cg, cb) = led::hsv_to_rgb(hue, 1.0, 1.0);
        for &g in gpus {
            led::set_color(nv, g, cr, cg, cb, r.brightness)?;
        }
        hue = (hue + r.step_deg) % 360.0;
        sleep_interruptible(r.interval_ms, &stop);
    }
    println!("\nstopped.");
    Ok(())
}

fn sleep_interruptible(ms: u64, stop: &AtomicBool) {
    let mut left = ms;
    while left > 0 && !stop.load(Ordering::SeqCst) {
        let chunk = left.min(50);
        std::thread::sleep(Duration::from_millis(chunk));
        left -= chunk;
    }
}
