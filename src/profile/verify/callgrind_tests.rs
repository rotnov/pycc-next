use super::*;

#[test]
fn baseline_deadline_is_a_multiple_of_cpython_wall_time_with_a_floor() {
    // A short workload gets the floor: valgrind's start dominates.
    assert_eq!(baseline_deadline(0), MIN_DEADLINE);
    assert_eq!(baseline_deadline(50_000_000), MIN_DEADLINE);
    // The longest workload `auto` counts, one second, gets 300 s.
    assert_eq!(baseline_deadline(1_000_000_000), Duration::from_secs(300));
    assert_eq!(baseline_deadline(2_000_000_000), Duration::from_secs(600));
    // No overflow on an absurd median.
    assert!(baseline_deadline(u64::MAX) >= MIN_DEADLINE);
}

#[test]
fn check_deadline_is_a_multiple_of_the_baseline_run_with_a_floor() {
    assert_eq!(check_deadline(Duration::from_secs(5)), MIN_DEADLINE);
    assert_eq!(check_deadline(Duration::from_secs(30)), MIN_DEADLINE);
    assert_eq!(check_deadline(Duration::from_secs(45)), Duration::from_secs(180));
    assert_eq!(check_deadline(Duration::MAX), Duration::MAX);
}

#[cfg(unix)]
#[test]
fn output_within_returns_the_output_of_a_command_that_finishes() {
    let mut command = Command::new("sh");
    command.args(["-c", "echo out; echo err >&2; exit 3"]);
    let output = output_within(&mut command, Duration::from_secs(30)).unwrap().unwrap();
    assert_eq!(output.stdout, b"out\n");
    assert_eq!(output.stderr, b"err\n");
    assert_eq!(output.status.code(), Some(3));
}

#[cfg(unix)]
#[test]
fn output_within_kills_the_whole_tree_past_its_deadline() {
    let dir = std::env::temp_dir().join(format!("pycc-callgrind-deadline-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pid_file = dir.join("grandchild.pid");
    // The shell starts a grandchild that would outlive it, records its pid,
    // and waits on it: both run far past the deadline.
    let script = format!("sleep 60 & echo $! > '{}'; wait", pid_file.display());
    let mut command = Command::new("sh");
    command.args(["-c", &script]);
    let started = Instant::now();
    let output = output_within(&mut command, Duration::from_millis(300)).unwrap();
    assert!(output.is_none(), "a run past its deadline has no output");
    assert!(started.elapsed() < Duration::from_secs(10), "the run was killed at its deadline");
    let grandchild = std::fs::read_to_string(&pid_file).unwrap();
    let grandchild = grandchild.trim();
    // Killed, it is reaped by init: allow that a moment, then it must be gone.
    let alive = || Command::new("kill").args(["-0", grandchild]).stderr(Stdio::null()).status().unwrap().success();
    let gone = (0..100).any(|_| {
        let alive = alive();
        if alive {
            std::thread::sleep(Duration::from_millis(50));
        }
        !alive
    });
    let _ = std::fs::remove_dir_all(&dir);
    assert!(gone, "the grandchild {grandchild} outlived the deadline");
}

#[test]
fn output_within_reports_a_command_that_cannot_start() {
    let mut command = Command::new("/nonexistent/pycc-no-such-program");
    assert!(output_within(&mut command, Duration::from_secs(1)).is_err());
}
