use refact_lsp::chat::perf_harness::{self, BenchmarkOptions, HarnessMode};

fn main() {
    let mode = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [flag] if flag == "--quick" => HarnessMode::Quick,
        [flag] if flag == "--soak" => HarnessMode::Soak,
        _ => {
            eprintln!("usage: concurrent_chat_bench --quick | --soak");
            std::process::exit(2);
        }
    };
    let options = match mode {
        HarnessMode::Quick => BenchmarkOptions::quick(),
        HarnessMode::Soak => BenchmarkOptions::soak(),
    };
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
