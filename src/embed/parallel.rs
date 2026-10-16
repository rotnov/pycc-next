//! Copies of an embedded build's sidecar run on every core (v0.5 accept 4):
//! the standard library and a locked closure are a thousand or more files
//! each, read, hashed and written independently of each other.

/// `f(i)` for every `i` in `0..count`, split over the host's cores, in
/// index order.
pub(crate) fn map_indexed<R: Send>(count: usize, f: impl Fn(usize) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(count.max(1));
    if threads <= 1 {
        return (0..count).map(f).collect();
    }
    let f = &f;
    let chunk = count.div_ceil(threads);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|t| {
                let range = (t * chunk).min(count)..((t + 1) * chunk).min(count);
                (
                    range.clone(),
                    scope.spawn(move || range.map(f).collect::<Vec<R>>()),
                )
            })
            .collect();
        let mut out = Vec::with_capacity(count);
        for (range, worker) in workers {
            match worker.join() {
                Ok(results) => out.extend(results),
                // A panicking copy is a bug: rerun its share here, where
                // the panic propagates with its own message.
                Err(_) => out.extend(range.map(f)),
            }
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_index_is_mapped_once_in_order() {
        for count in [0, 1, 2, 7, 64, 1000] {
            let out = map_indexed(count, |i| i * 2);
            assert_eq!(out, (0..count).map(|i| i * 2).collect::<Vec<_>>());
        }
    }
}
