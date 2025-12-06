// wasmrun - Run WASM binary on a worker hart (auto-selects least loaded)
//
// Usage:
//   wasmrun <file>              Run WASM on least loaded hart
//   wasmrun <file> --wait       Wait for job to complete

#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate mkfs;

#[cfg(target_arch = "wasm32")]
mod wasm {
    use mkfs::{
        argc, argv, console_log, file_exists, get_job_status, get_worker_count,
        print_int, read_file, sleep, submit_wasm_job, JobStatus,
    };

    const MAX_WASM_SIZE: usize = 1024 * 1024; // 1MB max WASM binary

    #[no_mangle]
    pub extern "C" fn _start() {
        let arg_count = argc();

        if arg_count < 1 {
            print_usage();
            return;
        }

        // Parse arguments
        let mut wait_for_completion = false;
        let mut filename_idx: Option<usize> = None;

        let mut arg_buf = [0u8; 256];

        for i in 0..arg_count {
            if let Some(len) = argv(i, &mut arg_buf) {
                let arg = unsafe { core::str::from_utf8_unchecked(&arg_buf[..len]) };

                if arg == "--help" || arg == "-h" {
                    print_usage();
                    return;
                } else if arg == "--wait" || arg == "-w" {
                    wait_for_completion = true;
                } else if !arg.starts_with('-') && filename_idx.is_none() {
                    filename_idx = Some(i);
                }
            }
        }

        // Check workers available
        let worker_count = get_worker_count();
        if worker_count == 0 {
            console_log("\x1b[1;33mNote:\x1b[0m No WASM workers (single-hart mode)\n");
            console_log("WASM runs synchronously on primary hart.\n");
            console_log("Use the command directly instead of wasmrun.\n");
            return;
        }

        // Get filename
        let filename_idx = match filename_idx {
            Some(idx) => idx,
            None => {
                console_log("\x1b[1;31mError:\x1b[0m No filename specified\n");
                print_usage();
                return;
            }
        };

        // Re-read filename
        let mut file_buf = [0u8; 256];
        let file_len = match argv(filename_idx, &mut file_buf) {
            Some(len) => len,
            None => {
                console_log("\x1b[1;31mError:\x1b[0m Failed to read filename\n");
                return;
            }
        };
        let filename_str = unsafe { core::str::from_utf8_unchecked(&file_buf[..file_len]) };

        // Resolve path
        let (resolved_path, path_len) = resolve_wasm_path(filename_str);
        let resolved_str = unsafe { core::str::from_utf8_unchecked(&resolved_path[..path_len]) };

        // Check if file exists
        if !file_exists(resolved_str) {
            console_log("\x1b[1;31mError:\x1b[0m File not found: ");
            console_log(resolved_str);
            console_log("\n");
            return;
        }

        // Read WASM file
        let mut wasm_buf = [0u8; MAX_WASM_SIZE];
        let wasm_len = match read_file(resolved_str, &mut wasm_buf) {
            Some(len) => len,
            None => {
                console_log("\x1b[1;31mError:\x1b[0m Failed to read file\n");
                return;
            }
        };

        // Verify WASM magic
        if wasm_len < 4 || wasm_buf[0] != 0x00 || wasm_buf[1] != 0x61 
            || wasm_buf[2] != 0x73 || wasm_buf[3] != 0x6D {
            console_log("\x1b[1;31mError:\x1b[0m Not a valid WASM binary\n");
            return;
        }

        // Submit job (None = auto-select least loaded hart)
        console_log("\x1b[1;34m●\x1b[0m Submitting: ");
        console_log(filename_str);
        console_log(" → least loaded hart\n");

        let job_id = match submit_wasm_job(&wasm_buf[..wasm_len], "", None) {
            Some(id) => id,
            None => {
                console_log("\x1b[1;31mError:\x1b[0m Failed to submit job\n");
                return;
            }
        };

        console_log("\x1b[1;32m✓\x1b[0m Job ");
        print_int(job_id as i64);
        console_log(" queued\n");

        // Wait for completion if requested
        if wait_for_completion {
            console_log("Waiting...");
            loop {
                match get_job_status(job_id) {
                    Some(JobStatus::Completed) => {
                        console_log(" \x1b[1;32mdone\x1b[0m\n");
                        break;
                    }
                    Some(JobStatus::Failed) => {
                        console_log(" \x1b[1;31mfailed\x1b[0m\n");
                        break;
                    }
                    Some(JobStatus::Pending) | Some(JobStatus::Running) => {
                        sleep(100); // Poll every 100ms
                    }
                    None => {
                        console_log(" \x1b[1;31mnot found\x1b[0m\n");
                        break;
                    }
                }
            }
        }
    }

    fn print_usage() {
        console_log("\x1b[1;97mwasmrun\x1b[0m - Run WASM on a worker hart\n\n");
        console_log("Automatically selects the least loaded hart.\n\n");
        console_log("\x1b[1;33mUsage:\x1b[0m\n");
        console_log("  wasmrun <file>         Submit to least loaded hart\n");
        console_log("  wasmrun <file> --wait  Wait for completion\n\n");
        console_log("\x1b[1;33mExamples:\x1b[0m\n");
        console_log("  wasmrun hello          # Run /usr/bin/hello\n");
        console_log("  wasmrun cowsay --wait  # Run and wait for result\n\n");
        console_log("Use \x1b[1mhtop\x1b[0m to see hart load and worker status.\n");
    }

    fn resolve_wasm_path(path: &str) -> ([u8; 256], usize) {
        let mut result = [0u8; 256];
        let mut pos = 0;

        if path.starts_with('/') {
            // Absolute path
            for byte in path.bytes() {
                if pos < 256 {
                    result[pos] = byte;
                    pos += 1;
                }
            }
        } else {
            // Try /usr/bin/ first
            let prefix = b"/usr/bin/";
            for &byte in prefix {
                if pos < 256 {
                    result[pos] = byte;
                    pos += 1;
                }
            }
            for byte in path.bytes() {
                if pos < 256 {
                    result[pos] = byte;
                    pos += 1;
                }
            }
        }

        (result, pos)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
