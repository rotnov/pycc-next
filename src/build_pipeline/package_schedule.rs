//! The order a package build runs its member builds in (v0.8): in
//! parallel, each as soon as every member whose interface it reads
//! (`crate::package_build::producers`) has written it -- after that
//! member's frontend, before its codegen -- or has failed without one.

use std::path::PathBuf;
use std::process::Command;

/// One member build's outcome: its exit status, what it printed, and how
/// long it took.
pub(super) struct MemberRun {
    pub(super) ok: bool,
    pub(super) code: u8,
    pub(super) output: Vec<u8>,
    pub(super) seconds: f64,
}

/// Where member `index`'s interface appears.
fn interface(dirs: &[PathBuf], index: usize) -> PathBuf {
    dirs[index].join(crate::package_build::INTERFACE_NAME)
}

/// The members in the order they are started when ready: the ones the most
/// other members wait for, directly or through others, first, so no member
/// that others wait for queues behind one nothing waits for.
fn by_priority(producers: &[Vec<usize>]) -> Vec<usize> {
    let count = producers.len();
    // waiters[j]: the members reading j's interface.
    let mut waiters = vec![Vec::new(); count];
    for (member, its) in producers.iter().enumerate() {
        for &producer in its {
            waiters[producer].push(member);
        }
    }
    let weight = |start: usize| {
        let mut seen = vec![false; count];
        let mut stack = vec![start];
        let mut total = 0;
        while let Some(member) = stack.pop() {
            for &waiter in &waiters[member] {
                if !seen[waiter] {
                    seen[waiter] = true;
                    total += 1;
                    stack.push(waiter);
                }
            }
        }
        total
    };
    let weights: Vec<usize> = (0..count).map(weight).collect();
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by_key(|&member| std::cmp::Reverse(weights[member]));
    order
}

struct State {
    pending: Vec<usize>,
    finished: Vec<bool>,
}

/// Runs a member build for each of `dirs` (member `i` writes into
/// `dirs[i]`) with at most `jobs` at a time, starting member `i` once each
/// of `producers[i]` has written its interface or finished; `command(i,
/// available)` is its command, given the producers whose interfaces exist.
/// The outcomes, in member order.
pub(super) fn run_members(
    dirs: &[PathBuf],
    producers: &[Vec<usize>],
    jobs: usize,
    command: &(dyn Fn(usize, &[usize]) -> Command + Sync),
) -> Vec<Result<MemberRun, String>> {
    let state = std::sync::Mutex::new(State {
        pending: by_priority(producers),
        finished: vec![false; dirs.len()],
    });
    let results = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1) {
            scope.spawn(|| {
                loop {
                    let next = {
                        let mut state = state.lock().expect("the schedule lock");
                        if state.pending.is_empty() {
                            break;
                        }
                        let ready = state.pending.iter().position(|&member| {
                            producers[member].iter().all(|&producer| {
                                state.finished[producer] || interface(dirs, producer).exists()
                            })
                        });
                        ready.map(|position| state.pending.remove(position))
                    };
                    let Some(member) = next else {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                        continue;
                    };
                    let available: Vec<usize> = producers[member]
                        .iter()
                        .copied()
                        .filter(|&producer| interface(dirs, producer).exists())
                        .collect();
                    let started = std::time::Instant::now();
                    let outcome = command(member, &available)
                        .stdin(std::process::Stdio::null())
                        .output()
                        .map(|output| {
                            let mut text = output.stdout;
                            text.extend(output.stderr);
                            MemberRun {
                                ok: output.status.success(),
                                code: output.status.code().map_or(1, |c| c.clamp(1, 255) as u8),
                                output: text,
                                seconds: started.elapsed().as_secs_f64(),
                            }
                        })
                        .map_err(|e| format!("could not run a member build: {e}"));
                    state.lock().expect("the schedule lock").finished[member] = true;
                    results
                        .lock()
                        .expect("the results lock")
                        .push((member, outcome));
                }
            });
        }
    });
    let mut results = results.into_inner().expect("the results lock");
    results.sort_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, outcome)| outcome).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A command that writes its member's interface, then records the
    /// interfaces it saw, then exits with `fail`'s status.
    #[cfg(unix)]
    fn script(dir: &std::path::Path, seen: &[usize], fail: bool) -> Command {
        let mut cmd = Command::new("sh");
        let seen: Vec<String> = seen.iter().map(ToString::to_string).collect();
        cmd.arg("-c").arg(format!(
            "echo x > '{}'; echo '{}' > '{}'; exit {}",
            dir.join(crate::package_build::INTERFACE_NAME).display(),
            seen.join(","),
            dir.join("seen").display(),
            i32::from(fail)
        ));
        cmd
    }

    #[test]
    fn what_others_wait_for_starts_first() {
        assert_eq!(
            by_priority(&[vec![1, 2], vec![2], vec![], vec![0]]),
            [2, 1, 0, 3]
        );
        assert_eq!(by_priority(&[vec![], vec![]]), [0, 1]);
    }

    #[cfg(unix)]
    #[test]
    fn members_start_after_their_producers_interfaces() {
        let scratch = pycc_scratch::ScratchDir::new("package-schedule").unwrap();
        let dirs: Vec<PathBuf> = (0..4).map(|i| scratch.join(format!("m{i}"))).collect();
        for dir in &dirs {
            std::fs::create_dir_all(dir).unwrap();
        }
        let producers = [vec![1, 2], vec![2], vec![], vec![0]];
        let runs = run_members(&dirs, &producers, 3, &|member, available| {
            script(&dirs[member], available, member == 3)
        });
        assert!(runs.iter().take(3).all(|run| run.as_ref().unwrap().ok));
        assert!(!runs[3].as_ref().unwrap().ok);
        let seen = |i: usize| std::fs::read_to_string(dirs[i].join("seen")).unwrap();
        assert_eq!(seen(0), "1,2\n");
        assert_eq!(seen(1), "2\n");
        assert_eq!(seen(3), "0\n");
    }
}
