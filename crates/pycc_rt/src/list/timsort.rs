//! CPython 3.15's `list.sort` (`Objects/listobject.c`: timsort with the
//! powersort merge policy), ported step for step over element words.
//!
//! Only `<` is consulted, as in CPython. Any stable sort agrees with this
//! one while `<` is a consistent total order; when it is not -- a `nan`
//! among floats, whose comparisons are all false -- where the elements end
//! up depends on exactly which comparisons the algorithm makes, so the
//! natural-run detection (`count_run`), the binary insertion sort that
//! extends short runs to `minrun`, the merge order, and the galloping
//! merges (with their adaptive `min_gallop`) are all CPython's own.

/// `MIN_GALLOP`: consecutive wins of one run before a merge gallops.
const MIN_GALLOP: usize = 7;
/// `MAX_MINRUN`: the bound on the length a short run is extended to.
const MAX_MINRUN: usize = 64;

/// The comparison: CPython's `a < b` on two element words.
type Lt<'a> = &'a dyn Fn(i64, i64) -> bool;

/// A pending run: `a[base..base + len]`, and its powersort power.
#[derive(Clone, Copy)]
struct Run {
    base: usize,
    len: usize,
    power: u32,
}

/// CPython's `MergeState`, with `a` the slice being sorted.
struct Merge<'a> {
    a: &'a mut [i64],
    lt: Lt<'a>,
    min_gallop: usize,
    pending: Vec<Run>,
    mr_e: u32,
    mr_mask: usize,
    mr_current: usize,
}

/// Sorts `items` in place exactly as CPython's `list.sort` (no key, no
/// `reverse`) leaves a list whose elements compare with `lt`.
pub(crate) fn sort(items: &mut [i64], lt: Lt<'_>) {
    let n = items.len();
    if n < 2 {
        return;
    }
    let mut mr_e = 0;
    while n >> mr_e >= MAX_MINRUN {
        mr_e += 1;
    }
    let mut ms = Merge {
        a: items,
        lt,
        min_gallop: MIN_GALLOP,
        pending: Vec::new(),
        mr_e,
        mr_mask: (1usize << mr_e) - 1,
        mr_current: 0,
    };
    let mut lo = 0;
    let mut remaining = n;
    while remaining > 0 {
        let mut run = ms.count_run(lo, remaining);
        let minrun = ms.minrun_next();
        if run < minrun {
            let force = remaining.min(minrun);
            ms.binarysort(lo, force, run);
            run = force;
        }
        ms.found_new_run(run);
        ms.pending.push(Run {
            base: lo,
            len: run,
            power: 0,
        });
        lo += run;
        remaining -= run;
    }
    ms.merge_force_collapse();
}

/// Where a merge loop ended (`merge_lo`'s and `merge_hi`'s labels).
enum End {
    Succeed,
    /// One element of the run kept in place remains (`CopyB`/`CopyA`).
    CopyRest,
}

/// `gallop_left`: `k` with `a[k-1] < key <= a[k]`, searched from `hint`.
fn gallop_left(lt: Lt<'_>, key: i64, a: &[i64], hint: usize) -> usize {
    let n = a.len() as isize;
    let hint = hint as isize;
    let at = |i: isize| a[i as usize];
    let (mut lastofs, mut ofs): (isize, isize) = (0, 1);
    if lt(at(hint), key) {
        let maxofs = n - hint;
        while ofs < maxofs && lt(at(hint + ofs), key) {
            lastofs = ofs;
            ofs = (ofs << 1) + 1;
        }
        ofs = ofs.min(maxofs);
        lastofs += hint;
        ofs += hint;
    } else {
        let maxofs = hint + 1;
        while ofs < maxofs && !lt(at(hint - ofs), key) {
            lastofs = ofs;
            ofs = (ofs << 1) + 1;
        }
        ofs = ofs.min(maxofs);
        (lastofs, ofs) = (hint - ofs, hint - lastofs);
    }
    lastofs += 1;
    while lastofs < ofs {
        let m = lastofs + ((ofs - lastofs) >> 1);
        if lt(at(m), key) {
            lastofs = m + 1;
        } else {
            ofs = m;
        }
    }
    ofs as usize
}

/// `gallop_right`: `k` with `a[k-1] <= key < a[k]`, searched from `hint`.
fn gallop_right(lt: Lt<'_>, key: i64, a: &[i64], hint: usize) -> usize {
    let n = a.len() as isize;
    let hint = hint as isize;
    let at = |i: isize| a[i as usize];
    let (mut lastofs, mut ofs): (isize, isize) = (0, 1);
    if lt(key, at(hint)) {
        let maxofs = hint + 1;
        while ofs < maxofs && lt(key, at(hint - ofs)) {
            lastofs = ofs;
            ofs = (ofs << 1) + 1;
        }
        ofs = ofs.min(maxofs);
        (lastofs, ofs) = (hint - ofs, hint - lastofs);
    } else {
        let maxofs = n - hint;
        while ofs < maxofs && !lt(key, at(hint + ofs)) {
            lastofs = ofs;
            ofs = (ofs << 1) + 1;
        }
        ofs = ofs.min(maxofs);
        lastofs += hint;
        ofs += hint;
    }
    lastofs += 1;
    while lastofs < ofs {
        let m = lastofs + ((ofs - lastofs) >> 1);
        if lt(key, at(m)) {
            ofs = m;
        } else {
            lastofs = m + 1;
        }
    }
    ofs as usize
}

impl Merge<'_> {
    fn minrun_next(&mut self) -> usize {
        self.mr_current += self.a.len();
        let result = self.mr_current >> self.mr_e;
        self.mr_current &= self.mr_mask;
        result
    }

    /// `binarysort`: `a[lo..lo + n]` by binary insertion, `a[lo..lo + ok]`
    /// already sorted.
    fn binarysort(&mut self, lo: usize, n: usize, ok: usize) {
        let lt = self.lt;
        let s = &mut self.a[lo..lo + n];
        for ok in ok.max(1)..n {
            let pivot = s[ok];
            let (mut l, mut r) = (0, ok);
            while l < r {
                let m = (l + r) >> 1;
                if lt(pivot, s[m]) {
                    r = m;
                } else {
                    l = m + 1;
                }
            }
            s.copy_within(l..ok, l + 1);
            s[l] = pivot;
        }
    }

    /// `count_run`: the length of the run starting at `a[lo]` (at most
    /// `remaining`), made ascending in place.
    fn count_run(&mut self, lo: usize, remaining: usize) -> usize {
        let lt = self.lt;
        let s = &mut self.a[lo..lo + remaining];
        let mut n = 1;
        while n < remaining && !lt(s[n], s[n - 1]) {
            n += 1;
        }
        if n == remaining {
            return n;
        }
        if n > 1 {
            if lt(s[0], s[n - 1]) {
                return n;
            }
            s[..n].reverse();
        }
        n += 1;
        // The last `neq + 1` elements are all equal: reversed in place, so
        // the reversal of the whole run restores their order.
        let reverse_last_neq = |s: &mut [i64], n: usize, neq: &mut usize| {
            if *neq > 0 {
                s[n - *neq - 1..n].reverse();
                *neq = 0;
            }
        };
        let mut neq = 0;
        while n < remaining {
            if lt(s[n], s[n - 1]) {
                reverse_last_neq(s, n, &mut neq);
            } else if lt(s[n - 1], s[n]) {
                break;
            } else {
                neq += 1;
            }
            n += 1;
        }
        reverse_last_neq(s, n, &mut neq);
        s[..n].reverse();
        while n < remaining && !lt(s[n], s[n - 1]) {
            n += 1;
        }
        n
    }

    /// `merge_lo`: merges `a[pa..pa + na]` with the `nb` elements after
    /// it, `na <= nb`, through a copy of the first run.
    fn merge_lo(&mut self, pa: usize, mut na: usize, mut nb: usize) {
        let lt = self.lt;
        let tmp: Vec<i64> = self.a[pa..pa + na].to_vec();
        let a = &mut *self.a;
        let (mut dest, mut ia, mut ib) = (pa, 0, pa + na);
        a[dest] = a[ib];
        dest += 1;
        ib += 1;
        nb -= 1;
        let mut min_gallop = self.min_gallop;
        let end = 'merge: {
            if nb == 0 {
                break 'merge End::Succeed;
            }
            if na == 1 {
                break 'merge End::CopyRest;
            }
            loop {
                let (mut acount, mut bcount) = (0, 0);
                // One element at a time until one run wins consistently.
                loop {
                    if lt(a[ib], tmp[ia]) {
                        a[dest] = a[ib];
                        dest += 1;
                        ib += 1;
                        bcount += 1;
                        acount = 0;
                        nb -= 1;
                        if nb == 0 {
                            break 'merge End::Succeed;
                        }
                        if bcount >= min_gallop {
                            break;
                        }
                    } else {
                        a[dest] = tmp[ia];
                        dest += 1;
                        ia += 1;
                        acount += 1;
                        bcount = 0;
                        na -= 1;
                        if na == 1 {
                            break 'merge End::CopyRest;
                        }
                        if acount >= min_gallop {
                            break;
                        }
                    }
                }
                // Galloping, until neither run wins consistently.
                min_gallop += 1;
                loop {
                    min_gallop -= usize::from(min_gallop > 1);
                    self.min_gallop = min_gallop;
                    let k = gallop_right(lt, a[ib], &tmp[ia..ia + na], 0);
                    acount = k;
                    if k > 0 {
                        a[dest..dest + k].copy_from_slice(&tmp[ia..ia + k]);
                        dest += k;
                        ia += k;
                        na -= k;
                        if na == 1 {
                            break 'merge End::CopyRest;
                        }
                        // Impossible for a consistent `<`, but not assumed.
                        if na == 0 {
                            break 'merge End::Succeed;
                        }
                    }
                    a[dest] = a[ib];
                    dest += 1;
                    ib += 1;
                    nb -= 1;
                    if nb == 0 {
                        break 'merge End::Succeed;
                    }
                    let k = gallop_left(lt, tmp[ia], &a[ib..ib + nb], 0);
                    bcount = k;
                    if k > 0 {
                        a.copy_within(ib..ib + k, dest);
                        dest += k;
                        ib += k;
                        nb -= k;
                        if nb == 0 {
                            break 'merge End::Succeed;
                        }
                    }
                    a[dest] = tmp[ia];
                    dest += 1;
                    ia += 1;
                    na -= 1;
                    if na == 1 {
                        break 'merge End::CopyRest;
                    }
                    if acount < MIN_GALLOP && bcount < MIN_GALLOP {
                        break;
                    }
                }
                // Penalized for leaving galloping mode.
                min_gallop += 1;
                self.min_gallop = min_gallop;
            }
        };
        match end {
            End::Succeed => a[dest..dest + na].copy_from_slice(&tmp[ia..ia + na]),
            End::CopyRest => {
                // The last element of the first run belongs at the end.
                a.copy_within(ib..ib + nb, dest);
                a[dest + nb] = tmp[ia];
            }
        }
    }

    /// `merge_hi`: merges `a[pa..pa + na]` with the `nb` elements after
    /// it, `na >= nb`, from the right, through a copy of the second run.
    fn merge_hi(&mut self, pa: usize, mut na: usize, mut nb: usize) {
        let lt = self.lt;
        let pb = pa + na;
        let tmp: Vec<i64> = self.a[pb..pb + nb].to_vec();
        let a = &mut *self.a;
        // Signed: a cursor steps one before the start of its run.
        let mut dest = (pb + nb - 1) as isize;
        let mut ia = (pa + na - 1) as isize;
        let mut ib = nb as isize - 1;
        let u = |i: isize| i as usize;
        a[u(dest)] = a[u(ia)];
        dest -= 1;
        ia -= 1;
        na -= 1;
        let mut min_gallop = self.min_gallop;
        let end = 'merge: {
            if na == 0 {
                break 'merge End::Succeed;
            }
            if nb == 1 {
                break 'merge End::CopyRest;
            }
            loop {
                let (mut acount, mut bcount) = (0, 0);
                // One element at a time until one run wins consistently.
                loop {
                    if lt(tmp[u(ib)], a[u(ia)]) {
                        a[u(dest)] = a[u(ia)];
                        dest -= 1;
                        ia -= 1;
                        acount += 1;
                        bcount = 0;
                        na -= 1;
                        if na == 0 {
                            break 'merge End::Succeed;
                        }
                        if acount >= min_gallop {
                            break;
                        }
                    } else {
                        a[u(dest)] = tmp[u(ib)];
                        dest -= 1;
                        ib -= 1;
                        bcount += 1;
                        acount = 0;
                        nb -= 1;
                        if nb == 1 {
                            break 'merge End::CopyRest;
                        }
                        if bcount >= min_gallop {
                            break;
                        }
                    }
                }
                // Galloping, until neither run wins consistently.
                min_gallop += 1;
                loop {
                    min_gallop -= usize::from(min_gallop > 1);
                    self.min_gallop = min_gallop;
                    let k = na - gallop_right(lt, tmp[u(ib)], &a[pa..pa + na], na - 1);
                    acount = k;
                    if k > 0 {
                        dest -= k as isize;
                        ia -= k as isize;
                        a.copy_within(u(ia + 1)..u(ia + 1) + k, u(dest + 1));
                        na -= k;
                        if na == 0 {
                            break 'merge End::Succeed;
                        }
                    }
                    a[u(dest)] = tmp[u(ib)];
                    dest -= 1;
                    ib -= 1;
                    nb -= 1;
                    if nb == 1 {
                        break 'merge End::CopyRest;
                    }
                    let k = nb - gallop_left(lt, a[u(ia)], &tmp[..nb], nb - 1);
                    bcount = k;
                    if k > 0 {
                        dest -= k as isize;
                        ib -= k as isize;
                        let (from, to) = (u(ib + 1), u(dest + 1));
                        a[to..to + k].copy_from_slice(&tmp[from..from + k]);
                        nb -= k;
                        if nb == 1 {
                            break 'merge End::CopyRest;
                        }
                        // Impossible for a consistent `<`, but not assumed.
                        if nb == 0 {
                            break 'merge End::Succeed;
                        }
                    }
                    a[u(dest)] = a[u(ia)];
                    dest -= 1;
                    ia -= 1;
                    na -= 1;
                    if na == 0 {
                        break 'merge End::Succeed;
                    }
                    if acount < MIN_GALLOP && bcount < MIN_GALLOP {
                        break;
                    }
                }
                // Penalized for leaving galloping mode.
                min_gallop += 1;
                self.min_gallop = min_gallop;
            }
        };
        match end {
            End::Succeed => {
                let to = u(dest + 1) - nb;
                a[to..to + nb].copy_from_slice(&tmp[..nb]);
            }
            End::CopyRest => {
                // The first element of the second run belongs at the front.
                let (from, to) = (u(ia + 1) - na, u(dest + 1) - na);
                a.copy_within(from..from + na, to);
                a[to - 1] = tmp[u(ib)];
            }
        }
    }

    /// `merge_at`: merges the pending runs `i` and `i + 1`.
    fn merge_at(&mut self, i: usize) {
        let lt = self.lt;
        let (mut pa, mut na) = (self.pending[i].base, self.pending[i].len);
        let (pb, nb) = (self.pending[i + 1].base, self.pending[i + 1].len);
        self.pending[i].len = na + nb;
        self.pending.remove(i + 1);
        // Elements of the first run before the second's start stay put.
        let k = gallop_right(lt, self.a[pb], &self.a[pa..pa + na], 0);
        pa += k;
        na -= k;
        if na == 0 {
            return;
        }
        // Elements of the second run after the first's end stay put.
        let nb = gallop_left(lt, self.a[pa + na - 1], &self.a[pb..pb + nb], nb - 1);
        if nb == 0 {
            return;
        }
        if na <= nb {
            self.merge_lo(pa, na, nb);
        } else {
            self.merge_hi(pa, na, nb);
        }
    }

    /// `found_new_run`: the powersort merges a new run of length `n2`
    /// triggers before it is pushed.
    fn found_new_run(&mut self, n2: usize) {
        let Some(&Run { base, len, .. }) = self.pending.last() else {
            return;
        };
        let power = powerloop(base, len, n2, self.a.len());
        while self.pending.len() > 1 && self.pending[self.pending.len() - 2].power > power {
            self.merge_at(self.pending.len() - 2);
        }
        if let Some(last) = self.pending.last_mut() {
            last.power = power;
        }
    }

    /// `merge_force_collapse`: merges every pending run into one.
    fn merge_force_collapse(&mut self) {
        while self.pending.len() > 1 {
            let mut n = self.pending.len() - 2;
            if n > 0 && self.pending[n - 1].len < self.pending[n + 1].len {
                n -= 1;
            }
            self.merge_at(n);
        }
    }
}

/// `powerloop`: the power of the run `[s1, s1 + n1)` followed by one of
/// length `n2`, in a list of length `n`.
fn powerloop(s1: usize, n1: usize, n2: usize, n: usize) -> u32 {
    let mut result = 0;
    let mut a = 2 * s1 + n1;
    let mut b = a + n1 + n2;
    loop {
        result += 1;
        if a >= n {
            a -= n;
            b -= n;
        } else if b >= n {
            break;
        }
        a <<= 1;
        b <<= 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::sort;

    fn sorted_floats(xs: &[f64]) -> Vec<f64> {
        let mut words: Vec<i64> = xs.iter().map(|x| x.to_bits() as i64).collect();
        sort(&mut words, &|a, b| {
            f64::from_bits(a as u64) < f64::from_bits(b as u64)
        });
        words.iter().map(|w| f64::from_bits(*w as u64)).collect()
    }

    fn render(xs: &[f64]) -> String {
        let parts: Vec<String> = xs.iter().map(|x| format!("{x}")).collect();
        parts.join(" ")
    }

    #[test]
    fn a_nan_lands_where_cpython_leaves_it() {
        // `sorted(...)` under CPython 3.15.
        let cases: &[(&[f64], &str)] = &[
            (&[2.675, 0.0, f64::NAN], "0 NaN 2.675"),
            (&[f64::NAN, 758.92, 42.4], "42.4 NaN 758.92"),
            (&[3.0, f64::NAN, 1.0, 2.0], "3 NaN 1 2"),
            (&[3.0, 2.0, f64::NAN, 1.0, 0.5], "0.5 2 NaN 1 3"),
        ];
        for (input, want) in cases {
            assert_eq!(render(&sorted_floats(input)), *want, "{input:?}");
        }
    }

    #[test]
    fn sorts_long_inputs_stably_through_galloping_merges() {
        // Many runs past `MAX_MINRUN`, long stretches one run wins
        // (galloping, both merge directions) and many equal keys.
        let n = 5000usize;
        let keys: Vec<i64> = (0..n)
            .map(|i| ((i * 7919) % 613) as i64 / 3 + if i % 500 < 250 { 0 } else { 1000 })
            .collect();
        let mut words: Vec<i64> = (0..n as i64).collect();
        sort(&mut words, &|a, b| keys[a as usize] < keys[b as usize]);
        let mut want: Vec<i64> = (0..n as i64).collect();
        want.sort_by_key(|i| keys[*i as usize]);
        assert_eq!(words, want);
        let mut desc: Vec<i64> = (0..n as i64).rev().chain(0..100).collect();
        let mut want = desc.clone();
        want.sort();
        sort(&mut desc, &|a, b| a < b);
        assert_eq!(desc, want);
    }
}
