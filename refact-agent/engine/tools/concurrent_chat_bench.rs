use refact_lsp::chat::perf_harness::{self, BenchmarkOptions, HarnessMode};

fn main() {
    let mode = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [flag] if flag == "--quick" => HarnessMode::Quick,
        [flag] if flag == "--soak" => HarnessMode::Soak,
        [flag] if flag == "--full-soak" => HarnessMode::FullSoak,
        [flag] if flag == "--fanout" => {
            let report = match perf_harness::run_fanout_benchmark() {
                Ok(report) => report,
                Err(error) => {
                    eprintln!("fanout benchmark failed: {error}");
                    std::process::exit(1);
                }
            };
            match perf_harness::render_fanout_json(&report) {
                Ok(json) => println!("{json}"),
                Err(error) => {
                    eprintln!("fanout benchmark report failed: {error}");
                    std::process::exit(1);
                }
            }
            return;
        }
        _ => {
            eprintln!("usage: concurrent_chat_bench --quick | --soak | --full-soak | --fanout");
            std::process::exit(2);
        }
    };
    let options = match mode {
        HarnessMode::Quick => BenchmarkOptions::quick(),
        HarnessMode::Soak => BenchmarkOptions::soak(),
        HarnessMode::FullSoak => BenchmarkOptions::full_soak(),
    };
    if mode == HarnessMode::FullSoak {
        let report = match perf_harness::run_full_soak_benchmark(options) {
            Ok(report) => report,
            Err(error) => {
                eprintln!("full soak benchmark failed: {error}");
                std::process::exit(1);
            }
        };
        match perf_harness::render_full_soak_json(&report) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("full soak benchmark report failed: {error}");
                std::process::exit(1);
            }
        }
        return;
    }
    let report = match perf_harness::run_benchmark(options) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("benchmark failed: {error}");
            std::process::exit(1);
        }
    };
    match perf_harness::render_json(&report) {
        Ok(json) => println!("{json}"),
        Err(error) => {
            eprintln!("benchmark report failed: {error}");
            std::process::exit(1);
        }
    }
}
