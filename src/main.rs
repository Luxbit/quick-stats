mod benchmark;
mod helpers;
mod info;

use benchmark::{cpu::benchmark_cpu, gpu::benchmark_gpu};
use clap::{Arg, Command};
use info::cpu::get_cpu_info;
use info::gpu::get_gpu_info;
use info::network::{get_ping, get_public_ip, get_internet_speed};
use info::power::{get_battery_info, BatteryInfo};
use serde_json::{json, Value};
use std::fs::File;
use std::io::{self, Write};
use tch::Device;
use tokio::runtime::Runtime;

fn main() -> io::Result<()> {
    let matches = configure_cli();
    let output_format = matches.get_one::<String>("format").unwrap();
    let output_file = matches.get_one::<String>("outputFile");
    let features: Vec<&String> = matches.get_many::<String>("features").unwrap().collect();

    let mut cpu_info = None;
    let mut cpu_gflops = None;
    let mut cpu_elapsed_time = None;
    let mut battery_info = None;
    let mut gpu_results = None;
    let mut ping = None;
    let mut public_ip = None;
    let mut internet_speed = None;

    if features.contains(&&"cpu".to_string()) {
        let cpu_info_data = get_cpu_info();
        let (gflops, elapsed_time) = benchmark_cpu(5);
        cpu_info = Some(cpu_info_data);
        cpu_gflops = Some(gflops);
        cpu_elapsed_time = Some(elapsed_time);
    }

    if features.contains(&&"gpu".to_string()) {
        let supports_mps = cpu_info.as_ref().map_or(false, |info| {
            info.arch == Some("arm64".to_string()) && info.os == "macos"
        });
        gpu_results = if supports_mps {
            Some(benchmark_mps_gpu()?)
        } else {
            Some(benchmark_cuda_gpus()?)
        };
    }

    if features.contains(&&"battery".to_string()) {
        battery_info = Some(get_battery_info());
    }

    if features.contains(&&"network".to_string()) {
        ping = get_ping().ok();
        // Create a new Tokio runtime
        let rt = Runtime::new()?;
        // Use the runtime to block on the async function
        public_ip = rt.block_on(get_public_ip()).ok();
        internet_speed = rt.block_on(get_internet_speed()).ok();
    }

    let output = match output_format.as_str() {
        "json" => generate_json_output(
            cpu_info.as_ref(),
            cpu_gflops,
            cpu_elapsed_time,
            battery_info.as_ref(),
            gpu_results.as_ref(),
            ping,
            public_ip.as_ref(),
            internet_speed.as_ref(),
        )?,
        _ => generate_plain_output(
            cpu_info.as_ref(),
            cpu_gflops,
            cpu_elapsed_time,
            battery_info.as_ref(),
            gpu_results.as_ref(),
            ping,
            public_ip.as_ref(),
            internet_speed.as_ref(),
        ),
    };

    write_output(output_file, &output)
}

fn configure_cli() -> clap::ArgMatches {
    Command::new("System Benchmark")
        .version("1.0")
        .about("Benchmarks CPU and GPU performance, and provides battery information")
        .arg(
            Arg::new("format")
                .short('f')
                .long("format")
                .value_name("FORMAT")
                .help("Sets the output format: plain or json")
                .default_value("plain"),
        )
        .arg(
            Arg::new("outputFile")
                .short('o')
                .long("outputFile")
                .value_name("OUTPUT")
                .help("Specifies the file to write the output to"),
        )
        .arg(
            Arg::new("features")
                .short('e')
                .long("features")
                .value_name("FEATURE")
                .help("Select which benchmarks/features to run/enable: cpu, gpu, battery, network (comma-separated)")
                .default_value("cpu,gpu,battery,network")
                .use_value_delimiter(true),
        )
        .get_matches()
}
fn generate_json_output(
    cpu_info: Option<&info::cpu::CpuInfo>,
    cpu_gflops: Option<f64>,
    cpu_elapsed_time: Option<f64>,
    battery_info: Option<&BatteryInfo>,
    gpu_results: Option<&Vec<Value>>,
    ping: Option<u32>,
    public_ip: Option<&String>,
    internet_speed: Option<&(f64, f64)>,
) -> Result<String, serde_json::Error> {
    let mut output_json = json!({});

    if let Some(info) = cpu_info {
        // General information
        output_json["general"] = json!({
            "os": info.os,
            "os_version": info.os_version.as_deref().unwrap_or("Not available"),
        });

        // Memory information
        output_json["memory"] = json!({
            "total_memory_mb": info.total_memory,
            "used_memory_mb": info.used_memory,
            "total_swap_mb": info.total_swap,
            "used_swap_mb": info.used_swap,
        });

        // CPU-specific information
        output_json["cpu"] = json!({
            "arch": info.arch.as_deref().unwrap_or("Not available"),
            "cpu_count": info.cpu_count,
            "gflops": cpu_gflops.unwrap_or(0.0),
            "benchmark_duration_seconds": cpu_elapsed_time.unwrap_or(0.0),
        });
    }

    if let Some(gpu) = gpu_results {
        output_json["gpu"] = json!(gpu);
    }

    if let Some(battery) = battery_info {
        output_json["battery"] = json!({
            "has_battery": battery.has_battery,
            "charge_percent": battery.charge_percent,
            "is_charging": battery.is_charging,
            "wh_capacity": battery.wh_capacity,
        });
    }

    // Group network-related information
    let mut network = json!({});

    if let Some(ping_value) = ping {
        network["ping_ms"] = json!(ping_value);
    }

    if let Some(ip) = public_ip {
        network["public_ip"] = json!(ip);
    }

    if let Some((download, upload)) = internet_speed {
        network["speed"] = json!({
            "download_mbps": download,
            "upload_mbps": upload
        });
    }

    // Only add the network if it's not empty
    if !network.as_object().unwrap().is_empty() {
        output_json["network"] = network;
    }

    serde_json::to_string_pretty(&output_json)
}

fn generate_plain_output(
    cpu_info: Option<&info::cpu::CpuInfo>,
    cpu_gflops: Option<f64>,
    cpu_elapsed_time: Option<f64>,
    battery_info: Option<&BatteryInfo>,
    gpu_results: Option<&Vec<serde_json::Value>>,
    ping: Option<u32>,
    public_ip: Option<&String>,
    internet_speed: Option<&(f64, f64)>,
) -> String {
    let mut output = String::new();

    if let Some(info) = cpu_info {
        output.push_str(&format_general_info(
            info,
        ));
        output.push_str(&format_cpu_info(
            info,
            cpu_gflops.unwrap_or(0.0),
            cpu_elapsed_time.unwrap_or(0.0),
        ));
        output.push_str(&format_memory_info(
            info,
        ));
    }

    if let Some(gpu) = gpu_results {
        if let Some(supports_mps) = cpu_info
            .as_ref()
            .map(|info| info.arch == Some("arm64".to_string()) && info.os == "macos")
        {
            if supports_mps {
                output.push_str(&format_mps_gpu_info());
            } else {
                output.push_str(&format_cuda_gpus_info());
            }
        }
    }

    if let Some(battery) = battery_info {
        output.push_str(&format_battery_info(battery));
    }

    if let Some(p) = ping {
        output.push_str(&format!("=> Network:\nInternet Ping: {:.2} ms\n", p));
    }

    if let Some(ip) = public_ip {
        output.push_str(&format!("Public IP: {}\n", ip));
    }
    if let Some((download, upload)) = internet_speed {
        output.push_str(&format!("Download speed: {:.2} Mbps (minimum)\n", download));
        output.push_str(&format!("Upload speed: {:.2} Mbps (minimum)\n", upload));
    }
    output
}

fn format_general_info(cpu_info: &info::cpu::CpuInfo) -> String {
    format!(
        "=> General:\n\
        OS          : {}\n\
        OS version  : {}\n\n",
        cpu_info.os,
        cpu_info.os_version.as_deref().unwrap_or("Not available")
    )
}

fn format_memory_info(cpu_info: &info::cpu::CpuInfo) -> String {
    format!(
        "=> Memory:\n\
        Total       : {} mb\n\
        Used        : {} mb\n\
        Swap Total  : {} mb\n\
        Swap Used   : {} mb\n\n",
        cpu_info.total_memory,
        cpu_info.used_memory,
        cpu_info.total_swap,
        cpu_info.used_swap
    )
}

fn format_cpu_info(
    cpu_info: &info::cpu::CpuInfo,
    cpu_gflops: f64,
    cpu_elapsed_time: f64,
) -> String {
    format!(
        "=> CPU:\n\
        Architecture: {}\n\
        Count       : {}\n\
        FLOPS       : {:.2} GFLOPS\n\
        Benchmark duration: {:.2} seconds\n\n",
        cpu_info.arch.as_deref().unwrap_or("Not available"),
        cpu_info.cpu_count,
        cpu_gflops,
        cpu_elapsed_time
    )
}


fn format_battery_info(battery_info: &BatteryInfo) -> String {
    let charge = if battery_info.charge_percent.is_some() {
        format!("{}%", battery_info.charge_percent.unwrap().to_string())
    } else {
        "None".to_string()
    };
    let capacity = if battery_info.wh_capacity.is_some() {
        format!("{} Wh", battery_info.wh_capacity.unwrap().to_string())
    } else {
        "None".to_string()
    };

    format!(
        "=> Power:\n\
        Battery         : {:?}\n\
        State of charge : {}\n\
        Charging        : {:?}\n\
        Capacity        : {}\n\n",
        battery_info.has_battery,
        charge,
        battery_info.is_charging.unwrap_or(false),
        capacity
    )
}

fn benchmark_mps_gpu() -> io::Result<Vec<serde_json::Value>> {
    let (gpu_tflops, gpu_elapsed_time) = benchmark_gpu(Device::Mps, 1000);
    Ok(vec![json!({
        "device": "MPS",
        "tflops": gpu_tflops,
        "duration": gpu_elapsed_time,
    })])
}

fn benchmark_cuda_gpus() -> io::Result<Vec<serde_json::Value>> {
    let gpu_infos = get_gpu_info();
    let mut gpu_results = Vec::new();

    for (index, info) in gpu_infos.into_iter().enumerate() {
        let (gpu_tflops, gpu_elapsed_time) = benchmark_gpu(Device::Cuda(index), 1000);
        gpu_results.push(json!({
            "device_id": info.device_id,
            "device": format!("{:?}", info.device),
            "name": info.name.unwrap_or_else(|| "Not available".to_string()),
            "total_memory": info.total_memory.unwrap_or(0),
            "free_memory": info.free_memory.unwrap_or(0),
            "used_memory": info.used_memory.unwrap_or(0),
            "tflops": gpu_tflops,
            "duration": gpu_elapsed_time,
        }));
    }

    Ok(gpu_results)
}

fn format_mps_gpu_info() -> String {
    let (gpu_tflops, gpu_elapsed_time) = benchmark_gpu(Device::Mps, 1000);
    format!(
        "=> GPU:\n\
        GPU: integrated (MPS)\n\
        GPU FLOPS: {:.2} TFLOPS\n\
        GPU benchmark duration: {:.2} seconds\n\n",
        gpu_tflops, gpu_elapsed_time
    )
}

fn format_cuda_gpus_info() -> String {
    let gpu_infos = get_gpu_info();
    let mut output = String::new();

    for (index, info) in gpu_infos.into_iter().enumerate() {
        let (gpu_tflops, gpu_elapsed_time) = benchmark_gpu(Device::Cuda(index), 1000);
        output.push_str(&format!(
            "CUDA Device {} Information:\n\
            Device: {:?}\n\
            Name: {}\n\
            Total Memory: {}\n\
            Free Memory: {}\n\
            Used Memory: {}\n\
            GPU Estimated FLOPS: {:.2} TFLOPS\n\
            GPU benchmark duration: {:.2} seconds\n",
            info.device_id,
            info.device,
            info.name.unwrap_or_else(|| "Not available".to_string()),
            info.total_memory.unwrap_or(0),
            info.free_memory.unwrap_or(0),
            info.used_memory.unwrap_or(0),
            gpu_tflops,
            gpu_elapsed_time
        ));
    }

    output
}
fn write_output(output_file: Option<&String>, output: &str) -> io::Result<()> {
    if let Some(file_path) = output_file {
        let mut file = File::create(file_path)?;
        file.write_all(output.as_bytes())?;
    } else {
        println!("{}", output);
    }
    Ok(())
}
