use jwalk::WalkDir;
use rayon::ThreadPoolBuilder;
use rayon::prelude::*;
use std::collections::BTreeSet;
use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let start_dir = match env::current_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("Failed to determine current directory: {err}");
            std::process::exit(1);
        }
    };

    let workspaces = find_scarb_workspaces(&start_dir);

    if workspaces.is_empty() {
        println!("No Scarb workspaces found under {}.", start_dir.display());
        return;
    }

    println!("Found {} Scarb workspace(s):", workspaces.len());
    for workspace in &workspaces {
        println!("- {}", display_path(workspace, &start_dir));
    }

    if !ask_for_confirmation("\nRun `scarb clean` in all listed directories? [y/N]: ") {
        println!("Aborted.");
        return;
    }

    let workspace_list: Vec<_> = workspaces.iter().cloned().collect();
    let max_jobs = parse_jobs_from_env().unwrap_or_else(|| workspace_list.len().max(1));
    let jobs = max_jobs.min(workspace_list.len().max(1));

    println!("\nRunning `scarb clean` in parallel with up to {jobs} job(s)...");

    let pool = match ThreadPoolBuilder::new().num_threads(jobs).build() {
        Ok(pool) => pool,
        Err(err) => {
            eprintln!("Failed to create rayon thread pool: {err}");
            std::process::exit(1);
        }
    };

    let results = pool.install(|| {
        workspace_list
            .par_iter()
            .map(|workspace| {
                let manifest_path = workspace.join("Scarb.toml");
                let status = Command::new("scarb")
                    .arg("--manifest-path")
                    .arg(&manifest_path)
                    .arg("clean")
                    .env_remove("SCARB_MANIFEST_PATH")
                    .current_dir(workspace)
                    .status();

                (workspace, status)
            })
            .collect::<Vec<_>>()
    });

    let mut failures = 0usize;
    for (workspace, status) in results {
        match status {
            Ok(exit_status) if exit_status.success() => {
                println!("- {}: Success.", workspace.display());
            }
            Ok(exit_status) => {
                failures += 1;
                eprintln!("- {}: Failed with exit code: {exit_status}", workspace.display());
            }
            Err(err) => {
                failures += 1;
                eprintln!(
                    "- {}: Failed to execute `scarb clean`: {err}",
                    workspace.display()
                );
            }
        }
    }

    if failures == 0 {
        println!("\nDone. All workspaces cleaned successfully.");
    } else {
        eprintln!("\nDone with {failures} failure(s).");
        std::process::exit(1);
    }
}

fn find_scarb_workspaces(dir: &Path) -> BTreeSet<PathBuf> {
    let mut workspaces = BTreeSet::new();

    let walker =
        WalkDir::new(dir).process_read_dir(|_depth, _parent_path, _read_dir_state, children| {
            let has_scarb_toml = children.iter().any(|entry_result| {
                entry_result.as_ref().ok().is_some_and(|entry| {
                    entry
                        .path()
                        .file_name()
                        .is_some_and(|name| name == "Scarb.toml")
                        && entry.file_type().is_file()
                })
            });

            if has_scarb_toml {
                children.retain(|entry_result| {
                    entry_result
                        .as_ref()
                        .ok()
                        .is_none_or(|entry| !entry.file_type().is_dir())
                });
            }
        });

    for entry_result in walker {
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(err) => {
                eprintln!("Skipping unreadable path: {err}");
                continue;
            }
        };

        if entry.file_type().is_file()
            && entry
                .path()
                .file_name()
                .is_some_and(|name| name == "Scarb.toml")
        {
            if let Some(parent) = entry.path().parent() {
                workspaces.insert(parent.to_path_buf());
            }
        }
    }

    workspaces
}

fn ask_for_confirmation(prompt: &str) -> bool {
    print!("{prompt}");
    if let Err(err) = io::stdout().flush() {
        eprintln!("Failed to flush stdout: {err}");
        return false;
    }

    let mut input = String::new();
    if let Err(err) = io::stdin().read_line(&mut input) {
        eprintln!("Failed to read input: {err}");
        return false;
    }

    let normalized = input.trim().to_ascii_lowercase();
    normalized == "y" || normalized == "yes"
}

fn display_path(path: &Path, base: &Path) -> String {
    match path.strip_prefix(base) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Ok(rel) => rel.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

fn parse_jobs_from_env() -> Option<usize> {
    let raw = match env::var("SCARB_CLEAN_JOBS") {
        Ok(raw) => raw,
        Err(_) => return None,
    };

    match raw.trim().parse::<usize>() {
        Ok(0) => {
            eprintln!("Ignoring SCARB_CLEAN_JOBS=0, value must be >= 1.");
            None
        }
        Ok(value) => Some(value),
        Err(_) => {
            eprintln!("Ignoring invalid SCARB_CLEAN_JOBS value: {raw}");
            None
        }
    }
}
